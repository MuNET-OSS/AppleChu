use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

crate::config_section! {
    pub(crate) struct SkipMapAnimationConfig => SKIP_MAP_ANIMATION_CONFIG_SECTION {
        section: "SkipMapAnimation",
        order: 150,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "跳过地图动画",
        fields: {}
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<SkipMapAnimationConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "skip map animation",
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
