use std::ffi::{c_char, c_void, CStr};
use std::ptr;

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_ORDINAL_FLAG32: usize = 0x8000_0000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(address: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
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

/// 在指定模块的导入地址表中替换函数指针，返回替换前的原始函数地址。
///
/// # Safety
/// `module_base` 必须指向已加载的 32 位 PE 模块基址；调用方需保证替换后的函数签名与原函数一致。
pub unsafe fn hook_iat(
    module_base: usize,
    dll_name: &str,
    func_name: &str,
    new_func: *const (),
) -> Option<*const ()> {
    if module_base == 0 || new_func.is_null() {
        return None;
    }

    let dos = (module_base as *const ImageDosHeader).as_ref()?;
    if dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 {
        return None;
    }

    let nt = ((module_base + dos.e_lfanew as usize) as *const ImageNtHeaders32).as_ref()?;
    if nt.signature != IMAGE_NT_SIGNATURE {
        return None;
    }

    if nt.optional_header.number_of_rva_and_sizes <= IMAGE_DIRECTORY_ENTRY_IMPORT as u32 {
        return None;
    }

    let import_dir = nt.optional_header.data_directory[IMAGE_DIRECTORY_ENTRY_IMPORT];
    if import_dir.virtual_address == 0 {
        return None;
    }

    let mut desc = (module_base + import_dir.virtual_address as usize) as *const ImageImportDescriptor;
    while let Some(import) = desc.as_ref() {
        if import.name == 0 {
            break;
        }

        let imported_dll = CStr::from_ptr((module_base + import.name as usize) as *const c_char)
            .to_string_lossy();
        if imported_dll.eq_ignore_ascii_case(dll_name) {
            if let Some(original) = hook_import(module_base, import, func_name, new_func) {
                return Some(original);
            }
        }

        desc = desc.add(1);
    }

    None
}

unsafe fn hook_import(
    module_base: usize,
    import: &ImageImportDescriptor,
    func_name: &str,
    new_func: *const (),
) -> Option<*const ()> {
    let lookup_rva = if import.original_first_thunk != 0 {
        import.original_first_thunk
    } else {
        import.first_thunk
    };
    if lookup_rva == 0 || import.first_thunk == 0 {
        return None;
    }

    let mut lookup = (module_base + lookup_rva as usize) as *const usize;
    let mut iat = (module_base + import.first_thunk as usize) as *mut usize;
    while let Some(&lookup_value) = lookup.as_ref() {
        if lookup_value == 0 {
            break;
        }

        // 按序号导入没有函数名，不能用于本工具的名称匹配。
        if lookup_value & IMAGE_ORDINAL_FLAG32 == 0 {
            let import_by_name = (module_base + lookup_value) as *const ImageImportByName;
            let name = CStr::from_ptr(ptr::addr_of!((*import_by_name).name) as *const c_char).to_string_lossy();
            if name == func_name {
                return patch_iat_entry(iat, new_func);
            }
        }

        lookup = lookup.add(1);
        iat = iat.add(1);
    }

    None
}

unsafe fn patch_iat_entry(iat: *mut usize, new_func: *const ()) -> Option<*const ()> {
    let mut old_protect = 0;
    if VirtualProtect(
        iat.cast(),
        std::mem::size_of::<usize>(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return None;
    }

    let original = *iat as *const ();
    *iat = new_func as usize;

    let mut ignored = 0;
    let _ = VirtualProtect(iat.cast(), std::mem::size_of::<usize>(), old_protect, &mut ignored);
    Some(original)
}
