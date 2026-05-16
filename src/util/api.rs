use std::ffi::{c_char, c_void, CString};

use once_cell::sync::OnceCell;

#[repr(C)]
pub struct ChuModInfo {
    pub api_version: u32,
    pub loader_version: *const c_char,
    pub game_module: *const c_char,
    pub game_base: usize,
    pub game_size: u32,
    pub text_base: usize,
    pub text_size: u32,
    pub rdata_base: usize,
    pub rdata_size: u32,
    pub game_version: *const c_char,
}

#[repr(C)]
pub struct ChuModAPI {
    pub struct_size: u32,
    pub log: Option<unsafe extern "C" fn(*const c_char, ...)>,
    pub aob_scan: Option<unsafe extern "C" fn(usize, u32, *const u8, *const c_char) -> usize>,
    pub mem_read: Option<unsafe extern "C" fn(usize, *mut c_void, u32) -> i32>,
    pub mem_write: Option<unsafe extern "C" fn(usize, *const c_void, u32) -> i32>,
    pub mem_fill: Option<unsafe extern "C" fn(usize, u8, u32) -> i32>,
    pub hook_create: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32>,
    pub hook_enable: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub hook_disable: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub hook_remove: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub register_service: Option<unsafe extern "C" fn(*const c_char, *mut c_void) -> i32>,
    pub get_service: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
    pub publish: Option<unsafe extern "C" fn(*const c_char, *mut c_void, u32) -> i32>,
    pub subscribe: Option<unsafe extern "C" fn(*const c_char, Option<unsafe extern "C" fn(*const c_char, *mut c_void, u32)>) -> i32>,
    pub rtti_find_vtable: Option<unsafe extern "C" fn(*const c_char) -> usize>,
    pub config_get_int: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_get_float: Option<unsafe extern "C" fn(*const c_char, f32) -> f32>,
    pub config_get_bool: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_get_string: Option<unsafe extern "C" fn(*const c_char, *mut c_char, u32, *const c_char) -> i32>,
    pub config_set_int: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_set_float: Option<unsafe extern "C" fn(*const c_char, f32) -> i32>,
    pub config_set_bool: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_set_string: Option<unsafe extern "C" fn(*const c_char, *const c_char) -> i32>,
    pub log_info: Option<unsafe extern "C" fn(*const c_char)>,
    pub log_warn: Option<unsafe extern "C" fn(*const c_char)>,
    pub log_error: Option<unsafe extern "C" fn(*const c_char)>,
    pub log_path: *const c_char,
    pub toml_section_exists: Option<unsafe extern "C" fn(*const c_char) -> i32>,
    pub toml_get_bool: Option<unsafe extern "C" fn(*const c_char, *const c_char, i32) -> i32>,
    pub toml_get_int: Option<unsafe extern "C" fn(*const c_char, *const c_char, i32) -> i32>,
    pub toml_get_float: Option<unsafe extern "C" fn(*const c_char, *const c_char, f32) -> f32>,
    pub toml_get_string: Option<unsafe extern "C" fn(*const c_char, *const c_char, *mut c_char, u32, *const c_char) -> i32>,
    pub get_manifest_path: Option<unsafe extern "C" fn() -> *const c_char>,
    pub reload_mod: Option<unsafe extern "C" fn(*const c_char) -> i32>,
}

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
            .map_or(0, |func| unsafe { func(start, size, pattern.as_ptr(), mask.as_ptr()) })
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

    fn log(&self, msg: &str, select: impl FnOnce(&ChuModAPI) -> Option<unsafe extern "C" fn(*const c_char)>) {
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

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {{
        if let Some(api) = $crate::util::api::API.get() {
            api.log_info(&format!($($arg)*));
        }
    }};
}
