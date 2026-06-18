use crate::config::Config;
use crate::patch_engine::{PatchVariant, VersionedPatch, apply_patch};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "skip startup",
            section: "SkipStartup",
            variants: &[PatchVariant {
                pattern: Some("6A 07 8B CF E8 ?? ?? ?? ?? 6A 01 E8 ?? ?? ?? ?? 8B 35"),
                pattern_offset: 9,
                known_offsets: &[],
                expected: &[0x6A, 0x01],
                patch: &[0x6A, 0x04],
            }],
        },
    );
}
