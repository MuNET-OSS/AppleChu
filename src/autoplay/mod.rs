mod autoplay;

pub use autoplay::{is_enabled, was_used};

use crate::config::Config;
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct AutoplayConfig => AUTOPLAY_CONFIG_SECTION {
        section: "Autoplay",
        order: 180,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "自动游玩",
        fields: {
            pub hotkey: String = String::from("Home"),
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
    autoplay::init(api, &config.hotkey);
    autoplay::init_upload_guard(api);
}

pub fn is_config_enabled(config: &Config) -> bool {
    config
        .section::<AutoplayConfig>()
        .is_some_and(|config| config.enabled)
}

pub fn shutdown_all() {
    autoplay::shutdown_upload_guard();
    autoplay::shutdown();
}
