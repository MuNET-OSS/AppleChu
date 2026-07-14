use std::ffi::{c_char, c_void, CStr};

const PAGE_EXECUTE_READWRITE: u32 = 0x40;

pub unsafe extern "C" fn api_aob_scan(
    start: usize,
    size: u32,
    pat: *const u8,
    mask: *const c_char,
) -> usize {
    if mask.is_null() || pat.is_null() {
        return 0;
    }
    let mask_str = CStr::from_ptr(mask);
    let mask_bytes = mask_str.to_bytes();
    let len = mask_bytes.len();
    if (size as usize) < len {
        return 0;
    }
    for i in 0..=(size as usize - len) {
        let mem = start + i;
        let mut ok = true;
        for (j, &m) in mask_bytes.iter().enumerate() {
            if m == b'x' && *((mem + j) as *const u8) != *pat.add(j) {
                ok = false;
                break;
            }
        }
        if ok {
            return mem;
        }
    }
    0
}

pub unsafe extern "C" fn api_mem_read(addr: usize, buf: *mut c_void, size: u32) -> i32 {
    let mut old_protect = 0u32;
    if windows_sys_loader::Win32::System::Memory::VirtualProtect(
        addr as *const c_void,
        size as usize,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return -1;
    }
    std::ptr::copy_nonoverlapping(addr as *const u8, buf as *mut u8, size as usize);
    windows_sys_loader::Win32::System::Memory::VirtualProtect(
        addr as *const c_void,
        size as usize,
        old_protect,
        &mut old_protect,
    );
    0
}

pub unsafe extern "C" fn api_mem_write(addr: usize, buf: *const c_void, size: u32) -> i32 {
    let mut old_protect = 0u32;
    if windows_sys_loader::Win32::System::Memory::VirtualProtect(
        addr as *const c_void,
        size as usize,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return -1;
    }
    std::ptr::copy_nonoverlapping(buf as *const u8, addr as *mut u8, size as usize);
    windows_sys_loader::Win32::System::Memory::VirtualProtect(
        addr as *const c_void,
        size as usize,
        old_protect,
        &mut old_protect,
    );
    0
}

pub unsafe extern "C" fn api_mem_fill(addr: usize, value: u8, size: u32) -> i32 {
    let mut old_protect = 0u32;
    if windows_sys_loader::Win32::System::Memory::VirtualProtect(
        addr as *const c_void,
        size as usize,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return -1;
    }
    std::ptr::write_bytes(addr as *mut u8, value, size as usize);
    windows_sys_loader::Win32::System::Memory::VirtualProtect(
        addr as *const c_void,
        size as usize,
        old_protect,
        &mut old_protect,
    );
    0
}
