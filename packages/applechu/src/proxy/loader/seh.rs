use std::panic::{catch_unwind, AssertUnwindSafe};

use chu_abi::{
    ChuModAPI, ChuModFrameFunc, ChuModInfo, ChuModInitFunc, ChuModReadyFunc, ChuModShutdownFunc,
};

use super::log::{log_info, log_warn};

pub unsafe fn call_mod_init(
    name: &str,
    init: ChuModInitFunc,
    info: *const ChuModInfo,
    api: *const ChuModAPI,
) -> Option<i32> {
    match catch_unwind(AssertUnwindSafe(|| init(info, api))) {
        Ok(ret) => Some(ret),
        Err(_) => {
            super::crash_dump::log_panic_context("chumod_init", name);
            log_warn(&format!(
                "External module initialization panicked and was skipped: {name}"
            ));
            None
        }
    }
}

pub unsafe fn call_mod_shutdown(name: &str, shutdown: ChuModShutdownFunc) {
    if catch_unwind(AssertUnwindSafe(|| shutdown())).is_err() {
        super::crash_dump::log_panic_context("chumod_shutdown", name);
        log_warn(&format!("External module shutdown panicked: {name}"));
    }
}

pub unsafe fn call_mod_on_ready(name: &str, on_ready: ChuModReadyFunc) {
    if catch_unwind(AssertUnwindSafe(|| on_ready())).is_err() {
        super::crash_dump::log_panic_context("chumod_on_ready", name);
        log_info(&format!(
            "mod on_ready panic caught: {} (callback=chumod_on_ready)",
            name
        ));
    }
}

pub unsafe fn call_mod_on_frame(name: &str, on_frame: ChuModFrameFunc) {
    if catch_unwind(AssertUnwindSafe(|| on_frame())).is_err() {
        super::crash_dump::log_panic_context("chumod_on_frame", name);
        log_info(&format!(
            "mod on_frame panic caught: {} (callback=chumod_on_frame)",
            name
        ));
    }
}
