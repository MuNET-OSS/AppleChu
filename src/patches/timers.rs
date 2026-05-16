use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;
use crate::util::memory::{file_offset_to_va, write_value};

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "禁用选歌计时器",
            section: "DisableTimer",
            pattern: Some("32 C0"),
            pattern_offset: 0,
            known_offsets: &[0x3D4110],
            expected: &[0x32, 0xC0],
            patch: &[0xB0, 0x01],
        },
    );
    apply_custom_timers(api, config);
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "所有计时器 999",
            section: "AllTimers999",
            pattern: Some("69 44 24 04 E8 03 00 00"),
            pattern_offset: 0,
            known_offsets: &[0x8604F0],
            expected: &[0x69, 0x44, 0x24, 0x04, 0xE8, 0x03, 0x00, 0x00],
            patch: &[0xB8, 0x58, 0x3E, 0x0F, 0x00, 0x90, 0x90, 0x90],
        },
    );
}

fn apply_custom_timers(api: &Api, config: &Config) {
    if !config.is_enabled("CustomTimers") {
        return;
    }

    write_timer(
        api,
        "地图选择计时器",
        0x944658,
        config.get_int("CustomTimers", "map_select", 60),
    );
    write_timer(
        api,
        "票券选择计时器",
        0x939792,
        config.get_int("CustomTimers", "ticket_select", 60),
    );
    write_timer(
        api,
        "组曲选择计时器",
        0x9E68A1,
        config.get_int("CustomTimers", "course_select", 60),
    );
}

fn write_timer(api: &Api, name: &str, offset: u32, value: i64) {
    let Ok(value) = i8::try_from(value) else {
        api.log_warn(&format!("{} 数值超出 i8 范围，已跳过", name));
        return;
    };

    let addr = file_offset_to_va(api, offset);
    if addr == 0 {
        api.log_warn(&format!("{} 地址转换失败", name));
        return;
    }

    if write_value(api, addr, value) {
        api.log_info(&format!("补丁已应用: {} = {}", name, value));
    } else {
        api.log_warn(&format!("补丁写入失败: {}", name));
    }
}
