use std::ffi::{c_char, c_void, CString};
use std::ptr::NonNull;

use once_cell::sync::OnceCell;

pub use chu_abi::{ChuModAPI, ChuModInfo};

#[derive(Clone, Copy)]
pub struct Api {
    raw: NonNull<ChuModAPI>,
    info: RuntimeInfo,
}

#[derive(Clone, Copy)]
struct RuntimeInfo {
    game_base: usize,
    game_size: u32,
    text_base: usize,
    text_size: u32,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

pub static API: OnceCell<Api> = OnceCell::new();

impl Api {
    pub fn new(raw: *const ChuModAPI, info: *const ChuModInfo) -> Option<Self> {
        let raw = NonNull::new(raw.cast_mut())?;
        let info = unsafe { info.as_ref() }?;
        Some(Self {
            raw,
            info: RuntimeInfo {
                game_base: info.game_base,
                game_size: info.game_size,
                text_base: info.text_base,
                text_size: info.text_size,
            },
        })
    }

    pub fn log_info(&self, msg: &str) {
        self.log(msg, |api| api.log_info);
    }

    pub fn log_warn(&self, msg: &str) {
        self.log(msg, |api| api.log_warn);
    }

    pub fn log_error(&self, msg: &str) {
        self.log(msg, |api| api.log_error);
    }

    pub fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize {
        let Ok(mask) = CString::new(mask) else {
            return 0;
        };
        self.raw()
            .and_then(|api| api.aob_scan)
            .map_or(0, |func| unsafe {
                func(start, size, pattern.as_ptr(), mask.as_ptr())
            })
    }

    pub fn mem_write(&self, addr: usize, data: &[u8]) -> bool {
        let Ok(size) = u32::try_from(data.len()) else {
            return false;
        };
        self.raw()
            .and_then(|api| api.mem_write)
            .is_some_and(|func| unsafe { func(addr, data.as_ptr().cast(), size) == 0 })
    }

    pub fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool {
        let Ok(size) = u32::try_from(buf.len()) else {
            return false;
        };
        self.raw()
            .and_then(|api| api.mem_read)
            .is_some_and(|func| unsafe { func(addr, buf.as_mut_ptr().cast(), size) == 0 })
    }

    #[allow(dead_code)]
    pub fn mem_fill(&self, addr: usize, value: u8, size: u32) -> bool {
        self.raw()
            .and_then(|api| api.mem_fill)
            .is_some_and(|func| unsafe { func(addr, value, size) == 0 })
    }

    pub fn hook_create(&self, target: usize, detour: usize) -> Option<usize> {
        let mut trampoline = std::ptr::null_mut::<c_void>();
        let created = self
            .raw()
            .and_then(|api| api.hook_create)
            .is_some_and(|func| unsafe {
                func(
                    target as *mut c_void,
                    detour as *mut c_void,
                    &mut trampoline,
                ) == 0
            });
        created.then_some(trampoline as usize)
    }

    pub fn hook_enable(&self, target: usize) -> bool {
        self.raw()
            .and_then(|api| api.hook_enable)
            .is_some_and(|func| unsafe { func(target as *mut c_void) == 0 })
    }

    pub fn hook_disable(&self, target: usize) -> bool {
        self.raw()
            .and_then(|api| api.hook_disable)
            .is_some_and(|func| unsafe { func(target as *mut c_void) == 0 })
    }

    pub fn hook_remove(&self, target: usize) -> bool {
        self.raw()
            .and_then(|api| api.hook_remove)
            .is_some_and(|func| unsafe { func(target as *mut c_void) == 0 })
    }

    pub fn register_present_callback(&self, cb: chu_abi::ChuModPresentCallback) -> bool {
        self.raw()
            .and_then(|api| api.register_present_callback)
            .is_some_and(|func| unsafe { func(Some(cb)) == 0 })
    }

    #[allow(dead_code)]
    pub fn register_reset_callback(&self, cb: chu_abi::ChuModResetCallback) -> bool {
        self.raw()
            .and_then(|api| api.register_reset_callback)
            .is_some_and(|func| unsafe { func(Some(cb)) == 0 })
    }

    pub fn set_frame_lock(&self, fps: u32) -> bool {
        self.raw()
            .and_then(|api| api.set_frame_lock)
            .is_some_and(|func| unsafe { func(fps) == 0 })
    }

    pub fn game_base(&self) -> usize {
        self.info.game_base
    }

    pub fn game_size(&self) -> u32 {
        self.info.game_size
    }

    pub fn text_base(&self) -> usize {
        self.info.text_base
    }

    pub fn text_size(&self) -> u32 {
        self.info.text_size
    }

    fn raw(&self) -> Option<&ChuModAPI> {
        Some(unsafe { self.raw.as_ref() })
    }

    fn log(
        &self,
        msg: &str,
        select: impl FnOnce(&ChuModAPI) -> Option<unsafe extern "C" fn(*const c_char)>,
    ) {
        let Some(api) = self.raw() else {
            return;
        };
        let Some(func) = select(api) else {
            return;
        };
        if let Ok(msg) = CString::new(msg) {
            unsafe { func(msg.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_info_is_owned_after_crossing_ffi_boundary() {
        // Given: loader 提供的运行时信息只在初始化调用期间有效。
        let api = {
            let info = ChuModInfo {
                api_version: 4,
                loader_version: std::ptr::null(),
                game_module: std::ptr::null(),
                game_base: 0x1234,
                game_size: 0x5678,
                text_base: 0x9ABC,
                text_size: 0xDEF0,
                rdata_base: 0,
                rdata_size: 0,
            };

            // When: AppleChu 在 FFI 边界创建 API 句柄。
            Api::new(std::ptr::dangling(), &info).expect("有效指针必须被接受")
        };

        // Then: 句柄不再借用 loader 的临时 ChuModInfo。
        assert_eq!(api.game_base(), 0x1234);
        assert_eq!(api.game_size(), 0x5678);
        assert_eq!(api.text_base(), 0x9ABC);
        assert_eq!(api.text_size(), 0xDEF0);
    }
}
