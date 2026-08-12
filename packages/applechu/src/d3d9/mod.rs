mod fps_osd;
mod osd;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::util::api::Api;

static FPS_ENABLED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct D3dPresentParameters {
    back_buffer_width: u32,
    back_buffer_height: u32,
    back_buffer_format: u32,
    back_buffer_count: u32,
    multi_sample_type: u32,
    multi_sample_quality: u32,
    swap_effect: u32,
    device_window: usize,
    pub(crate) windowed: i32,
    enable_auto_depth_stencil: i32,
    auto_depth_stencil_format: u32,
    flags: u32,
    pub(crate) full_screen_refresh_rate_in_hz: u32,
    presentation_interval: u32,
}

impl D3dPresentParameters {
    pub(crate) fn force_windowed(&mut self) {
        self.windowed = 1;
        self.full_screen_refresh_rate_in_hz = 0;
    }
}

crate::config_section! {
    pub(crate) struct FpsDisplayConfig => FPS_DISPLAY_CONFIG_SECTION {
        section: "FpsDisplay",
        order: 320,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "display",
        comment: "FPS 显示",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct FrameLockConfig => FRAME_LOCK_CONFIG_SECTION {
        section: "FrameLock",
        order: 330,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "display",
        comment: "帧率锁定",
        fields: {
            pub fps: u32 = 60,
            min: 1,
            max: 240,
            comment: "目标帧率";
        }
    }
}

#[applechu_macros::config_section(stage = Late, order = 40)]
pub fn init_all(api: &Api, config: &Config) {
    let fps_enabled = config
        .section::<FpsDisplayConfig>()
        .is_some_and(|config| config.enabled);
    FPS_ENABLED.store(fps_enabled, Ordering::Relaxed);
    osd::set_fps_visible(fps_enabled);
    if fps_enabled || crate::autoplay::is_config_enabled(config) {
        if api.register_present_callback(on_present) {
            api.log_info("In-game overlay attached to the D3D9 frame callback");
        } else {
            api.log_warn("In-game overlay could not attach to the D3D9 frame callback");
        }
    }

    if let Some(config) = config
        .section::<FrameLockConfig>()
        .filter(|config| config.enabled)
    {
        let fps = config.fps;
        if fps > 0 {
            if api.set_frame_lock(fps) {
                api.log_info(&format!("Frame limit set to {fps} FPS"));
            } else {
                api.log_warn("Frame limit unavailable because the D3D9 interface is missing");
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

#[cfg(test)]
mod tests {
    use super::D3dPresentParameters;

    #[test]
    fn force_windowed_sets_required_d3d_fields() {
        // Given: 全屏展示参数带有非零刷新率。
        let mut parameters = D3dPresentParameters {
            windowed: 0,
            full_screen_refresh_rate_in_hz: 60,
            ..Default::default()
        };

        // When: 窗口模式在进入 D3D9 前被强制应用。
        parameters.force_windowed();

        // Then: D3D9 同时收到窗口标志与窗口模式要求的刷新率。
        assert_eq!(parameters.windowed, 1);
        assert_eq!(parameters.full_screen_refresh_rate_in_hz, 0);
    }
}
