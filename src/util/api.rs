use std::ffi::{CString, c_char, c_void};

use once_cell::sync::OnceCell;

pub use chu_abi::{ChuModAPI, ChuModInfo};

#[derive(Clone, Copy)]
pub struct Api {
    raw: *const ChuModAPI,
    info: *const ChuModInfo,
}

unsafe impl Send for Api {}
unsafe impl Sync for Api {}

pub static API: OnceCell<Api> = OnceCell::new();

impl Api {
    pub const fn new(raw: *const ChuModAPI, info: *const ChuModInfo) -> Self {
        Self { raw, info }
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
        self.info().map_or(0, |info| info.game_base)
    }

    pub fn game_size(&self) -> u32 {
        self.info().map_or(0, |info| info.game_size)
    }

    pub fn text_base(&self) -> usize {
        self.info().map_or(0, |info| info.text_base)
    }

    pub fn text_size(&self) -> u32 {
        self.info().map_or(0, |info| info.text_size)
    }

    fn raw(&self) -> Option<&ChuModAPI> {
        unsafe { self.raw.as_ref() }
    }

    fn info(&self) -> Option<&ChuModInfo> {
        unsafe { self.info.as_ref() }
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
