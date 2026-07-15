use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

crate::config_section! {
    pub(crate) struct FreePlayConfig => FREE_PLAY_CONFIG_SECTION {
        section: "FreePlay",
        order: 120,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "免费游玩",
        fields: {
            pub custom_text: String = String::new(),
            comment: "自定义 FREE PLAY 文本";
        }
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<FreePlayConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "free play",
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
