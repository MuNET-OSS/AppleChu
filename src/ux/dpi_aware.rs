use std::ffi::c_void;

use crate::config::Config;
use crate::util::api::Api;

type SetProcessDpiAwarenessContextFn = unsafe extern "system" fn(isize) -> i32;

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4isize;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> *const c_void;
}

pub fn init(api: &Api, config: &Config) {
    if !config.is_enabled("DpiAware") {
        return;
    }

    unsafe {
        let user32 = GetModuleHandleA(b"user32.dll\0".as_ptr());
        if user32 == 0 {
            api.log_warn("DPI awareness skipped: user32.dll not loaded");
            return;
        }

        let proc = GetProcAddress(user32, b"SetProcessDpiAwarenessContext\0".as_ptr());
        if proc.is_null() {
            api.log_info("DPI awareness skipped: Per-Monitor V2 not supported");
            return;
        }

        let set_process_dpi_awareness_context: SetProcessDpiAwarenessContextFn = std::mem::transmute(proc);
        if set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
            api.log_info("DPI awareness enabled: Per-Monitor V2");
        } else {
            api.log_warn("DPI awareness failed: SetProcessDpiAwarenessContext returned failure");
        }
    }
}
