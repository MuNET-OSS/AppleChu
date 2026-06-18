use std::sync::atomic::{AtomicI32, Ordering};

use crate::config::Config;
use crate::util::api::Api;

static MONITOR_INDEX: AtomicI32 = AtomicI32::new(0);

#[allow(dead_code)]
pub fn preferred_adapter() -> i32 {
    MONITOR_INDEX.load(Ordering::SeqCst)
}

pub fn init(api: &Api, config: &Config) {
    if !config.is_enabled("Window") {
        return;
    }

    let monitor = config.get_int("Window", "monitor", 0) as i32;
    if monitor > 0 {
        MONITOR_INDEX.store(monitor, Ordering::SeqCst);
        api.log_info(&format!("gfx: preferred monitor adapter = {}", monitor));
    }
}
