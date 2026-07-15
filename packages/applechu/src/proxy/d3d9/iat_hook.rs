use std::ffi::{c_char, c_void, CStr};
use std::ptr;

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_ORDINAL_FLAG32: usize = 0x8000_0000;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;

#[link(name = "kernel32")]
extern "system" {
    fn VirtualProtect(
        address: *mut c_void,
        size: usize,
        new_protect: u32,
        old_protect: *mut u32,
    ) -> i32;
}

#[repr(C)]
struct ImageDosHeader {
    e_magic: u16,
    e_pad: [u16; 29],
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
    head: [u8; 92],
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

    let mut desc =
        (module_base + import_dir.virtual_address as usize) as *const ImageImportDescriptor;
    while let Some(import) = desc.as_ref() {
        if import.name == 0 {
            break;
        }

        let imported_dll =
            CStr::from_ptr((module_base + import.name as usize) as *const c_char).to_string_lossy();
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

        if lookup_value & IMAGE_ORDINAL_FLAG32 == 0 {
            let import_by_name = (module_base + lookup_value) as *const ImageImportByName;
            let name = CStr::from_ptr(ptr::addr_of!((*import_by_name).name) as *const c_char)
                .to_string_lossy();
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
    let _ = VirtualProtect(
        iat.cast(),
        std::mem::size_of::<usize>(),
        old_protect,
        &mut ignored,
    );
    Some(original)
}

pub unsafe fn vtable_slot(instance: *mut c_void, index: usize) -> *mut usize {
    if instance.is_null() {
        return ptr::null_mut();
    }
    let vtable = *(instance as *const *mut usize);
    if vtable.is_null() {
        return ptr::null_mut();
    }
    vtable.add(index)
}

pub unsafe fn patch_slot(slot: *mut usize, value: usize) -> bool {
    if slot.is_null() || value == 0 {
        return false;
    }
    let mut old_protect = 0;
    if VirtualProtect(
        slot.cast(),
        std::mem::size_of::<usize>(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return false;
    }
    *slot = value;
    let mut ignored = 0;
    let _ = VirtualProtect(
        slot.cast(),
        std::mem::size_of::<usize>(),
        old_protect,
        &mut ignored,
    );
    true
}
