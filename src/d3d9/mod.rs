mod fps_osd;
mod osd;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::util::api::Api;

static FPS_ENABLED: AtomicBool = AtomicBool::new(false);

crate::config_section! {
    pub(crate) struct FpsDisplayConfig => FPS_DISPLAY_CONFIG_SECTION {
        section: "FpsDisplay",
        order: 210,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "FPS 显示",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct FrameLockConfig => FRAME_LOCK_CONFIG_SECTION {
        section: "FrameLock",
        order: 220,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "帧率锁定",
        fields: {
            pub fps: u32 = 60,
            comment: "目标帧率";
        }
    }
}

pub fn init_all(api: &Api, config: &Config) {
    let fps_enabled = config
        .section::<FpsDisplayConfig>()
        .is_some_and(|config| config.enabled);
    FPS_ENABLED.store(fps_enabled, Ordering::Relaxed);
    osd::set_fps_visible(fps_enabled);
    if fps_enabled || crate::autoplay::is_config_enabled(config) {
        if api.register_present_callback(on_present) {
            api.log_info("in-game OSD registered (loader d3d9 callback)");
        } else {
            api.log_warn("in-game OSD registration failed: loader d3d9 callback unavailable");
        }
    }

    if let Some(config) = config
        .section::<FrameLockConfig>()
        .filter(|config| config.enabled)
    {
        let fps = config.fps;
        if fps > 0 {
            if api.set_frame_lock(fps) {
                api.log_info(&format!("frame lock: {}fps (loader d3d9)", fps));
            } else {
                api.log_warn("frame lock failed: loader d3d9 API unavailable");
            }
        }
    }
}

unsafe extern "C" fn on_present(device: *mut c_void) {
    if FPS_ENABLED.load(Ordering::Relaxed) {
        fps_osd::on_present(device);
    }
    osd::render(device);
}
