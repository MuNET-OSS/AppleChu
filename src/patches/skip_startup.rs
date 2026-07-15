use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

crate::config_section! {
    pub(crate) struct SkipStartupConfig => SKIP_STARTUP_CONFIG_SECTION {
        section: "SkipStartup",
        order: 130,
        default_enabled: false,
        always_enabled: false,
        hidden: false,
        comment: "跳过启动画面",
        fields: {}
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<SkipStartupConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "skip startup",
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
