use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;
use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
use windows_sys::Win32::System::ProcessStatus::{
    K32EnumProcessModules, K32GetModuleInformation, MODULEINFO,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_ORDINAL_FLAG32: usize = 0x8000_0000;
const MAX_MODULES: usize = 1024;

pub struct HookSymbol {
    pub name: &'static str,
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
struct ImageFileHeader {
    machine: u16,
    number_of_sections: u16,
    time_date_stamp: u32,
    pointer_to_symbol_table: u32,
    number_of_symbols: u32,
    size_of_optional_header: u16,
    characteristics: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ImageDataDirectory {
    virtual_address: u32,
    size: u32,
}

#[repr(C)]
struct ImageOptionalHeader32 {
    magic: u16,
    major_linker_version: u8,
    minor_linker_version: u8,
    size_of_code: u32,
    size_of_initialized_data: u32,
    size_of_uninitialized_data: u32,
    address_of_entry_point: u32,
    base_of_code: u32,
    base_of_data: u32,
    image_base: u32,
    section_alignment: u32,
    file_alignment: u32,
    major_operating_system_version: u16,
    minor_operating_system_version: u16,
    major_image_version: u16,
    minor_image_version: u16,
    major_subsystem_version: u16,
    minor_subsystem_version: u16,
    win32_version_value: u32,
    size_of_image: u32,
    size_of_headers: u32,
    check_sum: u32,
    subsystem: u16,
    dll_characteristics: u16,
    size_of_stack_reserve: u32,
    size_of_stack_commit: u32,
    size_of_heap_reserve: u32,
    size_of_heap_commit: u32,
    loader_flags: u32,
    number_of_rva_and_sizes: u32,
    data_directory: [ImageDataDirectory; 16],
}

#[repr(C)]
struct ImageNtHeaders32 {
    signature: u32,
    file_header: ImageFileHeader,
    optional_header: ImageOptionalHeader32,
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

    if target_module != 0 {
        return hook_module(target_module as usize, dll_name, symbols);
    }

    let mut modules = [0; MAX_MODULES];
    let mut needed = 0u32;
    if K32EnumProcessModules(
        GetCurrentProcess(),
        modules.as_mut_ptr(),
        std::mem::size_of_val(&modules) as u32,
        &mut needed,
    ) == 0
    {
        return hook_current_exe(dll_name, symbols);
    }

    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());
    let kernel32 = GetModuleHandleA(b"kernel32.dll\0".as_ptr());
    let exe = GetModuleHandleA(ptr::null());
    let mut patched = 0;
    if exe != 0 && exe != kernel32 {
        patched += hook_module(exe as usize, dll_name, symbols);
    }
    for &module in &modules[..count] {
        if module != 0 && module != kernel32 && module != exe {
            patched += hook_module(module as usize, dll_name, symbols);
        }
    }
    patched
}

unsafe fn hook_current_exe(dll_name: &str, symbols: &[HookSymbol]) -> usize {
    let module = GetModuleHandleA(ptr::null());
    if module == 0 {
        0
    } else {
        hook_module(module as usize, dll_name, symbols)
    }
}

unsafe fn hook_module(module_base: usize, dll_name: &str, symbols: &[HookSymbol]) -> usize {
    if !is_valid_module(module_base) {
        return 0;
    }

    let Some(nt) = nt_headers(module_base) else {
        return 0;
    };
    if nt.optional_header.number_of_rva_and_sizes <= IMAGE_DIRECTORY_ENTRY_IMPORT as u32 {
        return 0;
    }

    let import_dir = nt.optional_header.data_directory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    if import_dir.virtual_address == 0 {
        return 0;
    }

    let mut patched = 0;
    let mut desc =
        (module_base + import_dir.virtual_address as usize) as *const ImageImportDescriptor;
    while let Some(import) = desc.as_ref() {
        if import.name == 0 {
            break;
        }

        let imported_dll =
            CStr::from_ptr((module_base + import.name as usize) as *const c_char).to_string_lossy();
        if dll_matches(&imported_dll, dll_name) {
            patched += hook_import(module_base, import, symbols);
        }

        desc = desc.add(1);
    }
    patched
}

unsafe fn is_valid_module(module_base: usize) -> bool {
    let mut info = MODULEINFO {
        lpBaseOfDll: ptr::null_mut::<c_void>(),
        SizeOfImage: 0,
        EntryPoint: ptr::null_mut::<c_void>(),
    };
    K32GetModuleInformation(
        GetCurrentProcess(),
        module_base as HMODULE,
        &mut info,
        std::mem::size_of::<MODULEINFO>() as u32,
    ) != 0
}

unsafe fn nt_headers(module_base: usize) -> Option<&'static ImageNtHeaders32> {
    let dos = (module_base as *const ImageDosHeader).as_ref()?;
    if dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 {
        return None;
    }

    let nt = ((module_base + dos.e_lfanew as usize) as *const ImageNtHeaders32).as_ref()?;
    (nt.signature == IMAGE_NT_SIGNATURE).then_some(nt)
}

unsafe fn hook_import(
    module_base: usize,
    import: &ImageImportDescriptor,
    symbols: &[HookSymbol],
) -> usize {
    let lookup_rva = if import.original_first_thunk != 0 {
        import.original_first_thunk
    } else {
        import.first_thunk
    };
    if lookup_rva == 0 || import.first_thunk == 0 {
        return 0;
    }

    let mut patched = 0;
    let mut lookup = (module_base + lookup_rva as usize) as *const usize;
    let mut iat = (module_base + import.first_thunk as usize) as *mut usize;
    while let Some(&lookup_value) = lookup.as_ref() {
        if lookup_value == 0 {
            break;
        }

        if lookup_value & IMAGE_ORDINAL_FLAG32 == 0 {
            let import_by_name = (module_base + lookup_value) as *const ImageImportByName;
            let name = CStr::from_ptr(ptr::addr_of!((*import_by_name).name) as *const c_char)
                .to_string_lossy();
            if let Some(symbol) = symbols.iter().find(|symbol| name == symbol.name) {
                if patch_iat_entry(iat, symbol) {
                    patched += 1;
                }
            }
        }

        lookup = lookup.add(1);
        iat = iat.add(1);
    }
    patched
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
    0
}
