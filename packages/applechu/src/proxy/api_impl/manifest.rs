use std::ffi::{c_char, CString};
use std::sync::Mutex;

static CURRENT_MANIFEST_PATH: once_cell::sync::Lazy<Mutex<Option<CString>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

pub fn set_current_manifest_path(path: Option<&str>) {
    if let Ok(mut current) = CURRENT_MANIFEST_PATH.lock() {
        *current = path.and_then(|p| CString::new(p).ok());
    }
}

pub unsafe extern "C" fn api_get_manifest_path() -> *const c_char {
    match CURRENT_MANIFEST_PATH.lock() {
        Ok(current) => current
            .as_ref()
            .map_or(std::ptr::null(), |path| path.as_ptr()),
        Err(_) => std::ptr::null(),
    }
}
