mod hotkeys;
mod restart_song;
mod skip_warning;

use crate::config::Config;
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct RestartSongConfig => RESTART_SONG_CONFIG_SECTION {
        section: "RestartSong",
        order: 341,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "gameplay",
        comment: "重新开始当前歌曲",
        fields: {
            pub hotkey: crate::config::VirtualKey = crate::config::VirtualKey::new(0x24),
            description: "重新开始当前歌曲",
            description_en: "Restart the current song",
            comment: "重开歌曲按键";
        }
    }
}

crate::config_section! {
    pub(crate) struct SkipWarningConfig => SKIP_WARNING_CONFIG_SECTION {
        section: "SkipWarning",
        order: 342,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "gameplay",
        comment: "跳过进入游戏前的警告",
        fields: {}
    }
}

#[applechu_macros::config_section(stage = Late, order = 50, shutdown = shutdown_all)]
pub fn init(api: &Api, config: &Config) -> Result<(), String> {
    if let Some(section) = config.section::<RestartSongConfig>() {
        if section.enabled {
            restart_song::init(api, section.hotkey.code())?;
        }
    }
    skip_warning::register_skip_warning(
        api,
        config
            .section::<SkipWarningConfig>()
            .is_some_and(|section| section.enabled),
    )
}

pub fn shutdown_all() {
    skip_warning::shutdown();
    restart_song::shutdown();
}
