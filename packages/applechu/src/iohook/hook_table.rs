use std::ffi::{c_char, c_void};
use std::ptr;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleA, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::ProcessStatus::{
    K32EnumProcessModules, K32GetModuleInformation, MODULEINFO,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_NT_OPTIONAL_HDR32_MAGIC: u16 = 0x010B;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x020B;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_ORDINAL_FLAG: usize = 1usize << (usize::BITS - 1);
const MAX_MODULES: usize = 1024;

pub struct HookSymbol {
    pub name: &'static str,
    pub patch: *const (),
    pub original: *mut *const (),
}

pub struct OrdinalHookSymbol {
    pub ordinal: u16,
    pub patch: *const (),
    pub original: *mut *const (),
}

#[repr(C)]
struct ImageDosHeader {
    e_magic: u16,
    e_cblp: u16,
    e_cp: u16,
    e_crlc: u16,
    e_cparhdr: u16,
    e_minalloc: u16,
    e_maxalloc: u16,
    e_ss: u16,
    e_sp: u16,
    e_csum: u16,
    e_ip: u16,
    e_cs: u16,
    e_lfarlc: u16,
    e_ovno: u16,
    e_res: [u16; 4],
    e_oemid: u16,
    e_oeminfo: u16,
    e_res2: [u16; 10],
    e_lfanew: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ImageDataDirectory {
    virtual_address: u32,
    size: u32,
}

#[repr(C)]
struct ImageImportDescriptor {
    original_first_thunk: u32,
    time_date_stamp: u32,
    forwarder_chain: u32,
    name: u32,
    first_thunk: u32,
}

#[repr(C)]
struct ImageImportByName {
    hint: u16,
    name: [c_char; 1],
}

pub unsafe fn hook_table_apply(
    target_module: HMODULE,
    dll_name: &str,
    symbols: &[HookSymbol],
) -> usize {
    if symbols.is_empty() {
        return 0;
    }

    if !target_module.is_null() {
        return hook_module(target_module.addr(), dll_name, symbols);
    }

    let mut modules: [HMODULE; MAX_MODULES] = [ptr::null_mut(); MAX_MODULES];
    let mut needed = 0u32;
    if K32EnumProcessModules(
        GetCurrentProcess(),
        modules.as_mut_ptr(),
        std::mem::size_of_val(&modules) as u32,
        &mut needed,
    ) == 0
    {
        log_diag("hook_table: module enumeration failed, falling back to current executable");
        return hook_current_exe(dll_name, symbols);
    }

    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
    let kernel32 = GetModuleHandleA(b"kernel32.dll\0".as_ptr());
    let exe = GetModuleHandleA(ptr::null());
    let self_module = current_hook_module();
    let mut patched = 0;
    if !exe.is_null() && exe != kernel32 {
        patched += hook_module(exe.addr(), dll_name, symbols);
    }
    for &module in &modules[..count] {
        if !module.is_null() && module != kernel32 && module != exe && module != self_module {
            patched += hook_module(module.addr(), dll_name, symbols);
        }
    }
    patched
}

fn log_diag(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}

pub unsafe fn hook_table_apply_ordinals(
    target_module: HMODULE,
    dll_name: &str,
    symbols: &[OrdinalHookSymbol],
) -> usize {
    if symbols.is_empty() {
        return 0;
    }
    if !target_module.is_null() {
        return hook_module_ordinals(target_module.addr(), dll_name, symbols);
    }

    let mut modules: [HMODULE; MAX_MODULES] = [ptr::null_mut(); MAX_MODULES];
    let mut needed = 0u32;
    if K32EnumProcessModules(
        GetCurrentProcess(),
        modules.as_mut_ptr(),
        std::mem::size_of_val(&modules) as u32,
        &mut needed,
    ) == 0
    {
        let module = GetModuleHandleA(ptr::null());
        return if module.is_null() {
            0
        } else {
            hook_module_ordinals(module.addr(), dll_name, symbols)
        };
    }

    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
    let kernel32 = GetModuleHandleA(b"kernel32.dll\0".as_ptr());
    let exe = GetModuleHandleA(ptr::null());
    let self_module = current_hook_module();
    let mut patched = 0;
    if !exe.is_null() && exe != kernel32 {
        patched += hook_module_ordinals(exe.addr(), dll_name, symbols);
    }
    for &module in &modules[..count] {
        if !module.is_null() && module != kernel32 && module != exe && module != self_module {
            patched += hook_module_ordinals(module.addr(), dll_name, symbols);
        }
    }
    patched
}

unsafe fn current_hook_module() -> HMODULE {
    let mut module = ptr::null_mut();
    let address = hook_table_apply as *const () as *const u16;
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if GetModuleHandleExW(flags, address, &mut module) == 0 {
        ptr::null_mut()
    } else {
        module
    }
}

unsafe fn hook_current_exe(dll_name: &str, symbols: &[HookSymbol]) -> usize {
    let module = GetModuleHandleA(ptr::null());
    if module.is_null() {
        0
    } else {
        hook_module(module.addr(), dll_name, symbols)
    }
}

unsafe fn hook_module(module_base: usize, dll_name: &str, symbols: &[HookSymbol]) -> usize {
    let Some(image_size) = module_image_size(module_base) else {
        return 0;
    };
    let Some(import_dir) = import_directory(module_base, image_size) else {
        return 0;
    };
    if import_dir.virtual_address == 0 {
        return 0;
    }
    if !image_range_valid(
        module_base,
        image_size,
        import_dir.virtual_address as usize,
        import_dir.size as usize,
    ) {
        return 0;
    }

    let mut patched = 0;
    let mut desc =
        (module_base + import_dir.virtual_address as usize) as *const ImageImportDescriptor;
    let mut descriptor_offset = 0usize;
    while image_range_valid(
        module_base,
        image_size,
        import_dir.virtual_address as usize + descriptor_offset,
        std::mem::size_of::<ImageImportDescriptor>(),
    ) {
        let import = &*desc;
        if import.name == 0 {
            break;
        }

        let Some(imported_dll) = module_cstr(module_base, image_size, import.name) else {
            desc = desc.add(1);
            descriptor_offset += std::mem::size_of::<ImageImportDescriptor>();
            continue;
        };
        if dll_matches(&imported_dll, dll_name) {
            patched += hook_import(module_base, image_size, import, symbols);
        }

        desc = desc.add(1);
        descriptor_offset += std::mem::size_of::<ImageImportDescriptor>();
    }
    patched
}

unsafe fn hook_module_ordinals(
    module_base: usize,
    dll_name: &str,
    symbols: &[OrdinalHookSymbol],
) -> usize {
    let Some(image_size) = module_image_size(module_base) else {
        return 0;
    };
    let Some(import_dir) = import_directory(module_base, image_size) else {
        return 0;
    };
    if import_dir.virtual_address == 0 {
        return 0;
    }
    if !image_range_valid(
        module_base,
        image_size,
        import_dir.virtual_address as usize,
        import_dir.size as usize,
    ) {
        return 0;
    }

    let mut patched = 0;
    let mut desc =
        (module_base + import_dir.virtual_address as usize) as *const ImageImportDescriptor;
    let mut descriptor_offset = 0usize;
    while image_range_valid(
        module_base,
        image_size,
        import_dir.virtual_address as usize + descriptor_offset,
        std::mem::size_of::<ImageImportDescriptor>(),
    ) {
        let import = &*desc;
        if import.name == 0 {
            break;
        }
        let Some(imported_dll) = module_cstr(module_base, image_size, import.name) else {
            desc = desc.add(1);
            descriptor_offset += std::mem::size_of::<ImageImportDescriptor>();
            continue;
        };
        if dll_matches(&imported_dll, dll_name) {
            patched += hook_import_ordinals(module_base, image_size, import, symbols);
        }
        desc = desc.add(1);
        descriptor_offset += std::mem::size_of::<ImageImportDescriptor>();
    }
    patched
}

unsafe fn module_image_size(module_base: usize) -> Option<usize> {
    let mut info = MODULEINFO {
        lpBaseOfDll: ptr::null_mut::<c_void>(),
        SizeOfImage: 0,
        EntryPoint: ptr::null_mut::<c_void>(),
    };
    let valid = K32GetModuleInformation(
        GetCurrentProcess(),
        crate::util::win32::handle_from_value(module_base),
        &mut info,
        std::mem::size_of::<MODULEINFO>() as u32,
    ) != 0;
    valid
        .then_some(info.SizeOfImage as usize)
        .filter(|size| *size != 0)
}

unsafe fn import_directory(module_base: usize, image_size: usize) -> Option<ImageDataDirectory> {
    let dos = (module_base as *const ImageDosHeader).as_ref()?;
    if dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 {
        return None;
    }

    if !image_range_valid(module_base, image_size, dos.e_lfanew as usize, 24) {
        return None;
    }
    let nt = module_base.checked_add(dos.e_lfanew as usize)?;
    if ptr::read_unaligned(nt as *const u32) != IMAGE_NT_SIGNATURE {
        return None;
    }
    let optional = nt.checked_add(24)?;
    let (count_offset, directory_offset) = match ptr::read_unaligned(optional as *const u16) {
        IMAGE_NT_OPTIONAL_HDR32_MAGIC => (92usize, 96usize),
        IMAGE_NT_OPTIONAL_HDR64_MAGIC => (108usize, 112usize),
        _ => return None,
    };
    if !image_range_valid(
        module_base,
        image_size,
        dos.e_lfanew as usize + 24 + count_offset,
        4,
    ) {
        return None;
    }
    let count = ptr::read_unaligned((optional + count_offset) as *const u32);
    if count <= IMAGE_DIRECTORY_ENTRY_IMPORT as u32 {
        return None;
    }
    let directory_offset = dos.e_lfanew as usize
        + 24
        + directory_offset
        + IMAGE_DIRECTORY_ENTRY_IMPORT * std::mem::size_of::<ImageDataDirectory>();
    image_range_valid(
        module_base,
        image_size,
        directory_offset,
        std::mem::size_of::<ImageDataDirectory>(),
    )
    .then(|| ptr::read_unaligned((module_base + directory_offset) as *const ImageDataDirectory))
}

unsafe fn hook_import(
    module_base: usize,
    image_size: usize,
    import: &ImageImportDescriptor,
    symbols: &[HookSymbol],
) -> usize {
    // OriginalFirstThunk 为空时 FirstThunk 已经是函数地址，不能当作 RVA 读取
    let lookup_rva = import.original_first_thunk;
    if lookup_rva == 0 || import.first_thunk == 0 {
        return 0;
    }

    let mut patched = 0;
    let mut lookup = (module_base + lookup_rva as usize) as *const usize;
    let mut iat = (module_base + import.first_thunk as usize) as *mut usize;
    let mut index = 0usize;
    while image_range_valid(
        module_base,
        image_size,
        lookup_rva as usize + index * std::mem::size_of::<usize>(),
        std::mem::size_of::<usize>(),
    ) && image_range_valid(
        module_base,
        image_size,
        import.first_thunk as usize + index * std::mem::size_of::<usize>(),
        std::mem::size_of::<usize>(),
    ) {
        let lookup_value = *lookup;
        if lookup_value == 0 {
            break;
        }

        if lookup_value & IMAGE_ORDINAL_FLAG == 0 {
            let Some(name) = module_import_name(module_base, image_size, lookup_value) else {
                lookup = lookup.add(1);
                iat = iat.add(1);
                index += 1;
                continue;
            };
            if let Some(symbol) = symbols.iter().find(|symbol| name == symbol.name) {
                if patch_iat_entry(iat, symbol) {
                    patched += 1;
                }
            }
        }

        lookup = lookup.add(1);
        iat = iat.add(1);
        index += 1;
    }
    patched
}

unsafe fn hook_import_ordinals(
    module_base: usize,
    image_size: usize,
    import: &ImageImportDescriptor,
    symbols: &[OrdinalHookSymbol],
) -> usize {
    let lookup_rva = import.original_first_thunk;
    if lookup_rva == 0 || import.first_thunk == 0 {
        return 0;
    }

    let mut patched = 0;
    let mut lookup = (module_base + lookup_rva as usize) as *const usize;
    let mut iat = (module_base + import.first_thunk as usize) as *mut usize;
    let mut index = 0usize;
    while image_range_valid(
        module_base,
        image_size,
        lookup_rva as usize + index * std::mem::size_of::<usize>(),
        std::mem::size_of::<usize>(),
    ) && image_range_valid(
        module_base,
        image_size,
        import.first_thunk as usize + index * std::mem::size_of::<usize>(),
        std::mem::size_of::<usize>(),
    ) {
        let lookup_value = *lookup;
        if lookup_value == 0 {
            break;
        }
        if lookup_value & IMAGE_ORDINAL_FLAG != 0 {
            let ordinal = (lookup_value & 0xFFFF) as u16;
            if let Some(symbol) = symbols.iter().find(|symbol| symbol.ordinal == ordinal) {
                if patch_ordinal_iat_entry(iat, symbol) {
                    patched += 1;
                }
            }
        }
        lookup = lookup.add(1);
        iat = iat.add(1);
        index += 1;
    }
    patched
}

unsafe fn module_cstr(module_base: usize, image_size: usize, rva: u32) -> Option<String> {
    let offset = rva as usize;
    if offset >= image_size {
        return None;
    }
    let bytes =
        std::slice::from_raw_parts((module_base + offset) as *const u8, image_size - offset);
    let length = bytes.iter().position(|byte| *byte == 0)?;
    Some(String::from_utf8_lossy(&bytes[..length]).into_owned())
}

unsafe fn module_import_name(module_base: usize, image_size: usize, rva: usize) -> Option<String> {
    if rva & IMAGE_ORDINAL_FLAG != 0 || rva > u32::MAX as usize {
        return None;
    }
    let offset = rva.checked_add(std::mem::size_of::<u16>())?;
    if offset > u32::MAX as usize {
        return None;
    }
    module_cstr(module_base, image_size, offset as u32)
}

fn image_range_valid(module_base: usize, image_size: usize, offset: usize, length: usize) -> bool {
    offset <= image_size
        && length <= image_size.saturating_sub(offset)
        && module_base
            .checked_add(offset)
            .and_then(|address| address.checked_add(length))
            .is_some()
}

unsafe fn patch_ordinal_iat_entry(iat: *mut usize, symbol: &OrdinalHookSymbol) -> bool {
    if iat.is_null() || symbol.patch.is_null() {
        return false;
    }
    let mut old_protect = 0;
    if VirtualProtect(
        iat.cast(),
        std::mem::size_of::<usize>(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return false;
    }
    let original = *iat as *const ();
    if !symbol.original.is_null() && (*symbol.original).is_null() {
        *symbol.original = original;
    }
    *iat = symbol.patch as usize;
    let mut ignored = 0;
    let _ = VirtualProtect(
        iat.cast(),
        std::mem::size_of::<usize>(),
        old_protect,
        &mut ignored,
    );
    true
}

unsafe fn patch_iat_entry(iat: *mut usize, symbol: &HookSymbol) -> bool {
    if iat.is_null() || symbol.patch.is_null() {
        return false;
    }

    let mut old_protect = 0;
    if VirtualProtect(
        iat.cast(),
        std::mem::size_of::<usize>(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return false;
    }

    let original = *iat as *const ();
    if !symbol.original.is_null() && (*symbol.original).is_null() {
        *symbol.original = original;
    }
    *iat = symbol.patch as usize;

    let mut ignored = 0;
    let _ = VirtualProtect(
        iat.cast(),
        std::mem::size_of::<usize>(),
        old_protect,
        &mut ignored,
    );
    true
}

fn dll_matches(imported: &str, requested: &str) -> bool {
    imported.eq_ignore_ascii_case(requested)
        || (requested.eq_ignore_ascii_case("kernel32.dll") && is_kernel32_apiset(imported))
}

fn is_kernel32_apiset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("api-ms-win-core-") || lower.starts_with("ext-ms-win-")
}

pub fn null_module() -> HMODULE {
    ptr::null_mut()
}
