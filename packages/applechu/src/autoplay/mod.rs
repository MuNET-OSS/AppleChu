#[cfg(target_arch = "x86")]
mod autoplay;

#[cfg(target_arch = "x86")]
pub use autoplay::{is_enabled, was_used};

#[cfg(not(target_arch = "x86"))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(target_arch = "x86"))]
pub fn was_used() -> bool {
    false
}

use crate::config::Config;
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct AutoplayConfig => AUTOPLAY_CONFIG_SECTION {
        section: "Autoplay",
        order: 180,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "gameplay",
        description: "只屏蔽成绩数据，角色/设置/地图进度正常保存",
        description_en: "Only blocks score data, settings/progress saved normally",
        comment: "自动游玩",
        fields: {
            pub hotkey: String = String::from("Home"),
            description: "支持 Home、Insert、F1 或虚拟键码",
            description_en: "Supports Home, Insert, F1, or a virtual-key code",
            comment: "自动游玩切换按键";
        }
    }
}

#[applechu_macros::config_section(
    stage = Late,
    order = 20,
    shutdown = shutdown_all
)]
pub fn init_all(api: &Api, config: &AutoplayConfig) {
    #[cfg(target_arch = "x86")]
    {
        autoplay::init(api, &config.hotkey);
        autoplay::init_upload_guard(api);
    }
    #[cfg(not(target_arch = "x86"))]
    let _ = (api, config);
}

pub fn is_config_enabled(config: &Config) -> bool {
    config
        .section::<AutoplayConfig>()
        .is_some_and(|config| config.enabled)
}

pub fn shutdown_all() {
    #[cfg(target_arch = "x86")]
    {
        autoplay::shutdown_upload_guard();
        autoplay::shutdown();
    }
}
