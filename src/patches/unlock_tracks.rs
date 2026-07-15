use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;
use crate::util::pattern;

crate::config_section! {
    pub(crate) struct UnlockTracksConfig => UNLOCK_TRACKS_CONFIG_SECTION {
        section: "UnlockTracks",
        order: 225,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "解锁曲数上限",
        fields: {
            pub max: i32 = 3,
            comment: "每局最大曲数";
        }
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    let Some(config) = config
        .section::<UnlockTracksConfig>()
        .filter(|config| config.enabled)
    else {
        return;
    };
    apply_unlock_limit(api);
    apply_max_tracks(api, config.max);
}

fn apply_unlock_limit<M: PatchMemory>(api: &M) {
    apply_patch(
        api,
        &VersionedPatch {
            name: "unlock track limit",
            variants: &[PatchVariant {
                pattern: Some("B8 09 00 00 00 3B F0 5F 0F 47"),
                pattern_offset: 10,
                known_offsets: &[],
                expected: &[0xF0],
                patch: &[0xC0],
            }],
        },
    );
}

fn apply_max_tracks<M: PatchMemory>(api: &M, max_tracks: i32) {
    // B8 03 00 00 00 C3 = MOV EAX, 3; RET
    let addr = pattern::scan(api, "B8 03 00 00 00 C3");
    if addr == 0 {
        api.log_warn("max tracks: target function not found");
        return;
    }

    if api.mem_write(addr + 1, &max_tracks.to_le_bytes()) {
        api.log_info(&format!("patch applied: max tracks = {}", max_tracks));
    } else {
        api.log_warn("patch write failed: max tracks");
    }
}
