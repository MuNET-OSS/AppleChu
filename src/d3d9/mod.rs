mod fps_osd;
mod osd;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::util::api::Api;

static FPS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn init_all(api: &Api, config: &Config) {
    if config.is_enabled("DeviceLostFix") {
        api.log_info("device lost fix is loader-managed");
    }

    let fps_enabled = config.is_enabled("FpsDisplay");
    FPS_ENABLED.store(fps_enabled, Ordering::Relaxed);
    osd::set_fps_visible(fps_enabled);
    if fps_enabled || config.is_enabled("Autoplay") {
        if api.register_present_callback(on_present) {
            api.log_info("in-game OSD registered (loader d3d9 callback)");
        } else {
            api.log_warn("in-game OSD registration failed: loader d3d9 callback unavailable");
        }
    }

    if config.is_enabled("FrameLock") {
        let fps = config.get_int("FrameLock", "fps", 0) as u32;
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
