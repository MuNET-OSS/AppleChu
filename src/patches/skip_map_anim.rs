use crate::config::Config;
use crate::patch_engine::{PatchVariant, VersionedPatch, apply_patch};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "skip map animation",
            section: "SkipMapAnimation",
            variants: &[PatchVariant {
                pattern: None,
                pattern_offset: 0,
                known_offsets: &[0x8D63DA],
                expected: &[0x01],
                patch: &[0x00],
            }],
        },
    );
}
