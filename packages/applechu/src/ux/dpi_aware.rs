use std::ffi::c_void;

use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct DpiAwareConfig => DPI_AWARE_CONFIG_SECTION {
        section: "DpiAware",
        order: 180,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "启用 Per-Monitor V2 DPI 感知",
        fields: {}
    }
}

type SetProcessDpiAwarenessContextFn = unsafe extern "system" fn(isize) -> i32;

const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: isize = -4isize;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> *const c_void;
}

#[applechu_macros::config_section(stage = Late, order = 30)]
pub fn init(api: &Api, _config: &DpiAwareConfig) {
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

        let set_process_dpi_awareness_context: SetProcessDpiAwarenessContextFn =
            std::mem::transmute(proc);
        if set_process_dpi_awareness_context(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) != 0 {
            api.log_info("DPI awareness enabled: Per-Monitor V2");
        } else {
            api.log_warn("DPI awareness failed: SetProcessDpiAwarenessContext returned failure");
        }
    }
}
