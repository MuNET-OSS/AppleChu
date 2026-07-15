use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;
use crate::util::pattern;

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
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "disable song timer",
            section: "DisableTimer",
            variants: &[PatchVariant {
                pattern: Some("32 C0 C3"),
                pattern_offset: 0,
                known_offsets: &[],
                expected: &[0x32, 0xC0],
                patch: &[0xB0, 0x01],
            }],
        },
    );
}

fn apply_all_timers<M: PatchMemory>(api: &M, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "all timers 999",
            section: "AllTimers999",
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
    if !config.is_enabled("CustomTimers") {
        return;
    }

    apply_timer(
        api,
        config.get_int("CustomTimers", "map_select", 60),
        &TimerSite {
            name: "map select timer",
            pattern: "6A 01 8B CF E8 ?? ?? ?? ?? 68 84 03 00 00 6A 0A 6A ?? 8B CF E8",
            pattern_offset: 17,
        },
    );
    apply_timer(
        api,
        config.get_int("CustomTimers", "ticket_select", 60),
        &TimerSite {
            name: "ticket select timer",
            pattern: "6A 01 8B CE E8 ?? ?? ?? ?? 68 84 03 00 00 6A 0A 6A ?? 8B CE E8",
            pattern_offset: 17,
        },
    );
    apply_timer(
        api,
        config.get_int("CustomTimers", "course_select", 60),
        &TimerSite {
            name: "course select timer",
            pattern: "E8 ?? ?? ?? ?? 6A ?? E8 ?? ?? ?? ?? 83 C4 04 8D 4E 08 05 84 03 00 00",
            pattern_offset: 6,
        },
    );
}

fn apply_timer<M: PatchMemory>(api: &M, value: i64, site: &TimerSite) {
    let Ok(value) = i8::try_from(value) else {
        api.log_warn(&format!("{} value out of i8 range, skipped", site.name));
        return;
    };

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
