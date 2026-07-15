use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
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
