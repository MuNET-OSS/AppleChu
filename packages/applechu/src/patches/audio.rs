use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

crate::config_section! {
    pub(crate) struct ForceSharedAudioConfig => FORCE_SHARED_AUDIO_CONFIG_SECTION {
        section: "ForceSharedAudio",
        order: 270,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "强制共享音频，采样率必须为 48000Hz",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct Force2chAudioConfig => FORCE_2CH_AUDIO_CONFIG_SECTION {
        section: "Force2chAudio",
        order: 280,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "强制双声道",
        fields: {}
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    apply_shared_audio(api, config);
    apply_force_2ch(api, config);
}

fn apply_shared_audio<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<ForceSharedAudioConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "force shared audio",
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
    if !config
        .section::<Force2chAudioConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "force 2ch audio",
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
