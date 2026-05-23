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
            name: "unlock track limit",
            section: "UnlockTracks",
            pattern: Some("B8 09 00 00 00 3B F0 5F 0F 47"),
            pattern_offset: 10,
            known_offsets: &[],
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
    // B8 03 00 00 00 C3 = MOV EAX, 3; RET
    let addr = pattern::scan(api, "B8 03 00 00 00 C3");
    if addr == 0 {
        api.log_warn("max tracks: target function not found");
        return;
    }

    if write_value(api, addr + 1, max_tracks) {
        api.log_info(&format!("patch applied: max tracks = {}", max_tracks));
    } else {
        api.log_warn("patch write failed: max tracks");
    }
}
