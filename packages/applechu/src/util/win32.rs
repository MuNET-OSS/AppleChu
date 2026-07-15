use std::ffi::c_void;

use windows_sys::Win32::Foundation::HANDLE;

pub fn handle_from_value(value: usize) -> HANDLE {
    std::ptr::without_provenance_mut::<c_void>(value)
}

pub fn handle_value(handle: HANDLE) -> usize {
    handle.addr()
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
