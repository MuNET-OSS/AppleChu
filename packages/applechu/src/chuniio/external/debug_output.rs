use std::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::HMODULE;

use crate::util::api::API;
use crate::util::iat_hook::hook_iat;

type OutputDebugStringAFn = unsafe extern "system" fn(*const c_char);

static ORIGINAL_OUTPUT_DEBUG_STRING_A: AtomicUsize = AtomicUsize::new(0);

pub fn install(module: HMODULE) {
    // SAFETY: [Category 8 - FFI boundary] LoadLibraryW 已验证模块有效，detour 保持系统 ABI。
    let original = unsafe {
        hook_iat(
            module as usize,
            "KERNEL32.dll",
            "OutputDebugStringA",
            hooked_output_debug_string_a as *const (),
        )
    };
    if let Some(original) = original {
        ORIGINAL_OUTPUT_DEBUG_STRING_A.store(original as usize, Ordering::SeqCst);
        log_info("External ChuniIO debug output capture enabled");
    } else {
        log_info("External ChuniIO debug output capture unavailable");
    }
}

pub fn log_init_status(component: &str, status: i32) {
    let code = u32::from_ne_bytes(status.to_ne_bytes());
    let message = format!("External ChuniIO {component} initialization returned 0x{code:08X}");
    if status < 0 {
        if let Some(api) = API.get() {
            api.log_warn(&message);
        }
    } else {
        log_info(&message);
    }
}

unsafe extern "system" fn hooked_output_debug_string_a(message: *const c_char) {
    if !message.is_null() {
        // SAFETY: [Category 8 - FFI boundary] OutputDebugStringA 要求参数是有效的 NUL 结尾字符串。
        let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        for line in message.lines().filter(|line| !line.trim().is_empty()) {
            log_info(&format!("External ChuniIO: {}", line.trim()));
        }
    }

    let original = ORIGINAL_OUTPUT_DEBUG_STRING_A.load(Ordering::SeqCst);
    if original != 0 {
        // SAFETY: [Category 8 - FFI boundary] 地址来自原 IAT 槽位，签名与系统 ABI 未改变。
        let original: OutputDebugStringAFn = unsafe { std::mem::transmute(original) };
        unsafe { original(message) };
    }
}

fn log_info(message: &str) {
    if let Some(api) = API.get() {
        api.log_info(message);
    }
}
