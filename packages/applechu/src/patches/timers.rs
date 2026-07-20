use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;
use crate::util::pattern;

const DISABLE_TIMER_PATTERN: &str = concat!(
    "85 C0 74 ?? 83 F8 08 74 ?? E8 ?? ?? ?? ?? 3C 01 74 ?? ",
    "8B 0D ?? ?? ?? ?? E8 ?? ?? ?? ?? 8B C8 E8 ?? ?? ?? ?? ",
    "8D 48 78 E8 ?? ?? ?? ?? 3C 01 74 ?? 56 8D 8F A8 00 00 00 E8"
);
const DISABLE_TIMER_PATTERN_OFFSET: isize = 46;

crate::config_section! {
    pub(crate) struct DisableTimerConfig => DISABLE_TIMER_CONFIG_SECTION {
        section: "DisableTimer",
        order: 140,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "禁用选歌计时器",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct CustomTimersConfig => CUSTOM_TIMERS_CONFIG_SECTION {
        section: "CustomTimers",
        order: 200,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "自定义计时器",
        fields: {
            pub map_select: i8 = 60,
            comment: "地图选择计时";
            pub ticket_select: i8 = 60,
            comment: "票券选择计时";
            pub course_select: i8 = 60,
            comment: "课程选择计时";
        }
    }
}

crate::config_section! {
    pub(crate) struct AllTimers999Config => ALL_TIMERS_999_CONFIG_SECTION {
        section: "AllTimers999",
        order: 910,
        default_enabled: false,
        always_enabled: false,
        hidden: true,
        comment: "内部计时器诊断补丁",
        fields: {}
    }
}

struct TimerSite {
    name: &'static str,
    pattern: &'static str,
    pattern_offset: isize,
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    apply_disable_timer(api, config);
    apply_custom_timers(api, config);
    apply_all_timers(api, config);
}

fn apply_disable_timer<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<DisableTimerConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "disable song timer",
            variants: &[PatchVariant {
                pattern: Some(DISABLE_TIMER_PATTERN),
                pattern_offset: DISABLE_TIMER_PATTERN_OFFSET,
                known_offsets: &[0x9BE10D, 0x9DCEED, 0x9DD9BD],
                expected: &[0x74],
                patch: &[0xEB],
            }],
        },
    );
}

fn apply_all_timers<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<AllTimers999Config>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "all timers 999",
            variants: &[PatchVariant {
                pattern: Some("69 44 24 04 E8 03 00 00"),
                pattern_offset: 0,
                known_offsets: &[],
                expected: &[0x69, 0x44, 0x24, 0x04, 0xE8, 0x03, 0x00, 0x00],
                patch: &[0xB8, 0x58, 0x3E, 0x0F, 0x00, 0x90, 0x90, 0x90],
            }],
        },
    );
}

fn apply_custom_timers<M: PatchMemory>(api: &M, config: &Config) {
    let Some(config) = config
        .section::<CustomTimersConfig>()
        .filter(|config| config.enabled)
    else {
        return;
    };

    apply_timer(
        api,
        config.map_select,
        &TimerSite {
            name: "map select timer",
            pattern: "6A 01 8B CF E8 ?? ?? ?? ?? 68 84 03 00 00 6A 0A 6A ?? 8B CF E8",
            pattern_offset: 17,
        },
    );
    apply_timer(
        api,
        config.ticket_select,
        &TimerSite {
            name: "ticket select timer",
            pattern: "6A 01 8B CE E8 ?? ?? ?? ?? 68 84 03 00 00 6A 0A 6A ?? 8B CE E8",
            pattern_offset: 17,
        },
    );
    apply_timer(
        api,
        config.course_select,
        &TimerSite {
            name: "course select timer",
            pattern: "E8 ?? ?? ?? ?? 6A ?? E8 ?? ?? ?? ?? 83 C4 04 8D 4E 08 05 84 03 00 00",
            pattern_offset: 6,
        },
    );
}

fn apply_timer<M: PatchMemory>(api: &M, value: i8, site: &TimerSite) {
    let found = pattern::scan(api, site.pattern);
    let addr = if found == 0 {
        None
    } else {
        found.checked_add_signed(site.pattern_offset)
    };
    let Some(addr) = addr else {
        api.log_warn(&format!("{}: not found", site.name));
        return;
    };
    if api.mem_write(addr, &value.to_le_bytes()) {
        api.log_info(&format!("patch applied: {} = {}", site.name, value));
    } else {
        api.log_warn(&format!("patch write failed: {}", site.name));
    }
}
