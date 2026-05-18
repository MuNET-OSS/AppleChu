use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;
use crate::util::memory::write_value;
use crate::util::pattern;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "禁用选歌计时器",
            section: "DisableTimer",
            pattern: Some("32 C0 C3"),
            pattern_offset: 0,
            known_offsets: &[],
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
            known_offsets: &[],
            expected: &[0x69, 0x44, 0x24, 0x04, 0xE8, 0x03, 0x00, 0x00],
            patch: &[0xB8, 0x58, 0x3E, 0x0F, 0x00, 0x90, 0x90, 0x90],
        },
    );
}

fn apply_custom_timers(api: &Api, config: &Config) {
    if !config.is_enabled("CustomTimers") {
        return;
    }

    let map_val = config.get_int("CustomTimers", "map_select", 60);
    let ticket_val = config.get_int("CustomTimers", "ticket_select", 60);
    let course_val = config.get_int("CustomTimers", "course_select", 60);

    // map / ticket: 68 84 03 00 00 6A 0A 6A xx = PUSH 0x384 / PUSH 0xA / PUSH <timer>
    let base = api.text_base();
    let size = api.text_size();
    let map_addr = pattern::scan_range(api, base, size, "68 84 03 00 00 6A 0A 6A");
    if map_addr != 0 {
        write_timer_at(api, "地图选择计时器", map_addr + 7, map_val);

        let ticket_addr = pattern::scan_range(api, map_addr + 8, size - ((map_addr + 8 - base) as u32), "68 84 03 00 00 6A 0A 6A");
        if ticket_addr != 0 {
            write_timer_at(api, "票券选择计时器", ticket_addr + 7, ticket_val);
        } else {
            api.log_warn("票券选择计时器: 未找到");
        }
    } else {
        api.log_warn("地图选择计时器: 未找到");
    }

    // course: E8 ?? ?? ?? ?? 6A xx E8 ?? ?? ?? ?? 83 C4 04 8D 4E 08 05 84 03 00 00
    let course_addr = pattern::scan_range(api, base, size, "E8 ?? ?? ?? ?? 6A ?? E8 ?? ?? ?? ?? 83 C4 04 8D 4E 08 05 84 03 00 00");
    if course_addr != 0 {
        write_timer_at(api, "组曲选择计时器", course_addr + 6, course_val);
    } else {
        api.log_warn("组曲选择计时器: 未找到");
    }
}

fn write_timer_at(api: &Api, name: &str, addr: usize, value: i64) {
    let Ok(value) = i8::try_from(value) else {
        api.log_warn(&format!("{} 数值超出 i8 范围，已跳过", name));
        return;
    };

    if write_value(api, addr, value) {
        api.log_info(&format!("补丁已应用: {} = {}", name, value));
    } else {
        api.log_warn(&format!("补丁写入失败: {}", name));
    }
}
