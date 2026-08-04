use std::ffi::c_void;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{HANDLE, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

pub fn handle_from_value(value: usize) -> HANDLE {
    std::ptr::without_provenance_mut::<c_void>(value)
}

pub fn handle_value(handle: HANDLE) -> usize {
    handle.addr()
}

pub fn module_path(address: *const ()) -> Option<PathBuf> {
    unsafe {
        let module = module_handle(address)?;

        let mut path = vec![0u16; 32768];
        let length = GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) as usize;
        if length == 0 || length >= path.len() {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&path[..length])))
    }
}

pub fn module_base(address: *const ()) -> Option<usize> {
    unsafe { module_handle(address).map(|module| module as usize) }
}

unsafe fn module_handle(address: *const ()) -> Option<HMODULE> {
    let mut module: HMODULE = std::ptr::null_mut();
    if GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        address.cast(),
        &mut module,
    ) == 0
    {
        return None;
    }
    Some(module)
}

#[cfg(test)]
mod tests {
    use super::{handle_from_value, handle_value};

    #[test]
    fn opaque_handle_round_trips_its_value() {
        // Given: UART 仿真使用一个不会被 Rust 解引用的 opaque 句柄值。
        let value = 0x1234;

        // When: 该值跨越 windows-sys 0.59 的 HANDLE 边界。
        let handle = handle_from_value(value);

        // Then: 比较时可无损恢复原始值。
        assert_eq!(handle_value(handle), value);
    }
}
