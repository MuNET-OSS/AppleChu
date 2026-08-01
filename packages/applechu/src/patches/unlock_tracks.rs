use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;
use crate::util::pattern;

const MAX_TRACKS_PATTERN: &str = "B8 ?? ?? ?? ?? C3 CC CC CC CC CC CC CC CC CC CC 8B 44 24 04 53 B3 14 8B 10 85 D2 78 1E 83 FA 10";
const CLAMP_247_OFFSET: u32 = 0x6F8F92;
const CLAMP_250_OFFSET: u32 = 0x3DF06D;

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
            variants: &[
                PatchVariant {
                    pattern: None,
                    pattern_offset: 0,
                    known_offsets: &[CLAMP_250_OFFSET],
                    expected: &[0xB8, 0x07, 0, 0, 0, 0x3B, 0xC1, 0x0F, 0x47, 0xC1, 0xC3],
                    patch: &[0xB8, 0x63, 0, 0, 0, 0x3B, 0xC1, 0x90, 0x90, 0x90, 0xC3],
                },
                PatchVariant {
                    pattern: None,
                    pattern_offset: 0,
                    known_offsets: &[CLAMP_247_OFFSET],
                    expected: &[0xF0],
                    patch: &[0xC0],
                },
            ],
        },
    );
}

fn apply_max_tracks<M: PatchMemory>(api: &M, max_tracks: i32) {
    // 立即数可能已被硬补丁改写，后继函数前缀用于锁定真实曲数函数。
    let addr = pattern::scan(api, MAX_TRACKS_PATTERN);
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
