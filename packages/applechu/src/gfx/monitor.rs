use std::sync::atomic::{AtomicI32, Ordering};

use crate::gfx::WindowConfig;
use crate::util::api::Api;

static MONITOR_INDEX: AtomicI32 = AtomicI32::new(0);

#[allow(dead_code)]
pub fn preferred_adapter() -> i32 {
    MONITOR_INDEX.load(Ordering::SeqCst)
}

/// 配置的显示器编号越界时回退到主显示器
pub(crate) fn select_adapter(adapter_count: u32) -> u32 {
    let configured = preferred_adapter().max(0) as u32;
    if configured < adapter_count {
        configured
    } else {
        0
    }
}

#[applechu_macros::config_section(stage = Graphics, order = 20)]
pub fn init(api: &Api, config: &WindowConfig) {
    let monitor = config.monitor.max(0);
    MONITOR_INDEX.store(monitor, Ordering::SeqCst);
    if monitor > 0 {
        api.log_info(&format!("Game output assigned to monitor {monitor}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_adapter_falls_back_to_primary_when_out_of_range() {
        MONITOR_INDEX.store(2, Ordering::SeqCst);
        assert_eq!(select_adapter(3), 2);
        assert_eq!(select_adapter(2), 0);
        assert_eq!(select_adapter(0), 0);
        MONITOR_INDEX.store(0, Ordering::SeqCst);
    }
}
