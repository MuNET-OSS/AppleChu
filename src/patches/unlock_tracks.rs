use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;
use crate::util::memory::{file_offset_to_va, write_value};

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "解锁曲数上限",
            section: "UnlockTracks",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x6F8B82],
            expected: &[0xF0],
            patch: &[0xC0],
        },
    );
    apply_max_tracks(api, config);
}

fn apply_max_tracks(api: &Api, config: &Config) {
    if !config.is_enabled("UnlockTracks") {
        return;
    }

    let max_tracks = config.get_int("UnlockTracks", "max", 3) as i32;
    let addr = file_offset_to_va(api, 0x3EE331);
    if addr == 0 {
        api.log_warn("最大曲数地址转换失败");
        return;
    }

    if write_value(api, addr, max_tracks) {
        api.log_info(&format!("补丁已应用: 最大曲数 = {}", max_tracks));
    } else {
        api.log_warn("补丁写入失败: 最大曲数");
    }
}
