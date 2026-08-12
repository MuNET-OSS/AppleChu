use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

crate::config_section! {
    pub(crate) struct BypassAppUserConfig => BYPASS_APPUSER_CONFIG_SECTION {
        section: "BypassAppUser",
        order: 290,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "compatibility",
        comment: "绕过 AppUser 检测",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct Bypass120hzConfig => BYPASS_120HZ_CONFIG_SECTION {
        section: "Bypass120hz",
        order: 280,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "display",
        comment: "绕过 120Hz 检测",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct Bypass1080pConfig => BYPASS_1080P_CONFIG_SECTION {
        section: "Bypass1080p",
        order: 270,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "display",
        comment: "绕过 1080P 检测",
        fields: {}
    }
}

const BYPASS_1080P_EXPECTED: &[u8] = &[
    0x81, 0xBC, 0x24, 0x34, 0x02, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x75, 0x1F, 0x81, 0xBC, 0x24,
    0x38, 0x02, 0x00, 0x00, 0x38, 0x04, 0x00, 0x00, 0x75, 0x12,
];

const BYPASS_1080P_PATCH: &[u8] = &[
    0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
];

const APPUSER_250_EXPECTED: [u8; 34] = [
    0x83, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x1A, 0x56, 0xE8, 0x33, 0xE1, 0xFF, 0xFF, 0x8B, 0x70, 0x04,
    0xE8, 0x9B, 0xFF, 0xFF, 0xFF, 0x56, 0x8B, 0xC8, 0xE8, 0x13, 0xFF, 0xFF, 0xFF, 0x83, 0xC4, 0x04,
    0x5E, 0xC3,
];
const APPUSER_250_PATCH: [u8; 34] = patch_appuser_branch(APPUSER_250_EXPECTED);

const APPUSER_247_EXPECTED: [u8; 34] = [
    0x83, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x1A, 0x56, 0xE8, 0x13, 0xF4, 0xFF, 0xFF, 0x8B, 0x70, 0x04,
    0xE8, 0x9B, 0xFF, 0xFF, 0xFF, 0x56, 0x8B, 0xC8, 0xE8, 0x13, 0xFF, 0xFF, 0xFF, 0x83, 0xC4, 0x04,
    0x5E, 0xC3,
];
const APPUSER_247_PATCH: [u8; 34] = patch_appuser_branch(APPUSER_247_EXPECTED);

const fn patch_appuser_branch(mut bytes: [u8; 34]) -> [u8; 34] {
    bytes[5] = 0xEB;
    bytes
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    apply_bypass_1080p(api, config);
    apply_bypass_120hz(api, config);
    if !config
        .section::<BypassAppUserConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "bypass AppUser check",
            variants: &[
                PatchVariant {
                    pattern: None,
                    pattern_offset: 0,
                    known_offsets: &[0x8B3A0],
                    expected: &APPUSER_250_EXPECTED,
                    patch: &APPUSER_250_PATCH,
                },
                PatchVariant {
                    pattern: None,
                    pattern_offset: 0,
                    known_offsets: &[0x890C0],
                    expected: &APPUSER_247_EXPECTED,
                    patch: &APPUSER_247_PATCH,
                },
                PatchVariant {
                    pattern: None,
                    pattern_offset: 0,
                    known_offsets: &[0x89075],
                    expected: &[0x75],
                    patch: &[0xEB],
                },
                PatchVariant {
                    pattern: Some("83 7C 24 04 00 75"),
                    pattern_offset: 5,
                    known_offsets: &[],
                    expected: &[0x75],
                    patch: &[0xEB],
                },
            ],
        },
    );
}

fn apply_bypass_120hz<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<Bypass120hzConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "bypass 120Hz check",
            variants: &[PatchVariant {
                pattern: Some(
                    "85 C0 74 3F ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 81 BC 24 34 02 00 00 80 07 00 00",
                ),
                pattern_offset: 0,
                known_offsets: &[],
                expected: &[0x85, 0xC0, 0x74, 0x3F],
                patch: &[0xEB, 0x30, 0xEB, 0x2E],
            }],
        },
    );
}

fn apply_bypass_1080p<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<Bypass1080pConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }
    apply_patch(
        api,
        &VersionedPatch {
            name: "bypass 1080p check",
            variants: &[PatchVariant {
                pattern: Some(
                    "81 BC 24 34 02 00 00 80 07 00 00 75 1F 81 BC 24 38 02 00 00 38 04 00 00 75 12",
                ),
                pattern_offset: 0,
                known_offsets: &[],
                expected: BYPASS_1080P_EXPECTED,
                patch: BYPASS_1080P_PATCH,
            }],
        },
    );
}
