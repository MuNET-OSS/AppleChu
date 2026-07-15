use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    apply_shared_audio(api, config);
    apply_force_2ch(api, config);
}

fn apply_shared_audio<M: PatchMemory>(api: &M, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "force shared audio",
            section: "ForceSharedAudio",
            variants: &[PatchVariant {
                pattern: Some(
                    "6A 01 E8 ?? ?? ?? ?? 8D BE D0 00 00 00 0F 57 C0 0F 11 07 B8 FE FF 00 00",
                ),
                pattern_offset: 1,
                known_offsets: &[0xE29393, 0xE29EC3, 0xE5E663],
                expected: &[0x01],
                patch: &[0x00],
            }],
        },
    );
}

fn apply_force_2ch<M: PatchMemory>(api: &M, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "force 2ch audio",
            section: "Force2chAudio",
            variants: &[PatchVariant {
                pattern: Some("83 C4 04 85 C0 ?? ?? 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? B8 02 00 00 00"),
                pattern_offset: 5,
                known_offsets: &[0xE2944B, 0xE29F7B, 0xE5E71B],
                expected: &[0x75, 0x3F],
                patch: &[0x90, 0x90],
            }],
        },
    );
}
