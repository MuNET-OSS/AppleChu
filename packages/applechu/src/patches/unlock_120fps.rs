use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

crate::config_section! {
    pub(crate) struct Unlock120fpsConfig => UNLOCK_120FPS_CONFIG_SECTION {
        section: "Unlock120fps",
        order: 260,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "display",
        comment: "解锁 120fps",
        fields: {}
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<Unlock120fpsConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }

    // B9 78 00 00 00 B8 3C 00 00 00 0F 45 C1
    // = MOV ECX,120 / MOV EAX,60 / CMOVNZ EAX,ECX
    // 改 MOV EAX,60 → MOV EAX,120 强制 120fps
    apply_patch(
        api,
        &VersionedPatch {
            name: "unlock 120fps",
            variants: &[PatchVariant {
                pattern: Some("B9 78 00 00 00 B8 3C 00 00 00 0F 45 C1"),
                pattern_offset: 5,
                known_offsets: &[],
                expected: &[0xB8, 0x3C, 0x00, 0x00, 0x00],
                patch: &[0xB8, 0x78, 0x00, 0x00, 0x00],
            }],
        },
    );
}
