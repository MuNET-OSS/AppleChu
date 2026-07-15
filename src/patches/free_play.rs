use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
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
