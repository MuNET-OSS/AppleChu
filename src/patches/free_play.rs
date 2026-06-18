use crate::config::Config;
use crate::patch_engine::{PatchVariant, VersionedPatch, apply_patch};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "free play",
            section: "FreePlay",
            variants: &[PatchVariant {
                pattern: Some("E8 ?? ?? ?? ?? 3C 01 75 ?? 6A 09"),
                pattern_offset: 5,
                known_offsets: &[],
                expected: &[0x3C, 0x01],
                patch: &[0x38, 0xC0],
            }],
        },
    );
}
