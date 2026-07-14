use std::ffi::{c_char, CStr};

use crate::proxy::loader::log::{log_error, log_info, log_warn};

unsafe fn message_from_ptr(msg: *const c_char) -> Option<String> {
    if msg.is_null() {
        return None;
    }
    Some(CStr::from_ptr(msg).to_string_lossy().into_owned())
}

pub unsafe extern "C" fn api_log_info(msg: *const c_char) {
    if let Some(text) = message_from_ptr(msg) {
        log_info(&text);
    }
}

pub unsafe extern "C" fn api_log_warn(msg: *const c_char) {
    if let Some(text) = message_from_ptr(msg) {
        log_warn(&text);
    }
}

pub unsafe extern "C" fn api_log_error(msg: *const c_char) {
    if let Some(text) = message_from_ptr(msg) {
        log_error(&text);
    }
}
