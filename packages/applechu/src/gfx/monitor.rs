use std::sync::atomic::{AtomicI32, Ordering};

use crate::gfx::WindowConfig;
use crate::util::api::Api;

static MONITOR_INDEX: AtomicI32 = AtomicI32::new(0);

#[allow(dead_code)]
pub fn preferred_adapter() -> i32 {
    MONITOR_INDEX.load(Ordering::SeqCst)
}

#[applechu_macros::config_section(stage = Graphics, order = 20)]
pub fn init(api: &Api, config: &WindowConfig) {
    let monitor = config.monitor;
    if monitor > 0 {
        MONITOR_INDEX.store(monitor, Ordering::SeqCst);
        api.log_info(&format!("gfx: preferred monitor adapter = {}", monitor));
    }
}
