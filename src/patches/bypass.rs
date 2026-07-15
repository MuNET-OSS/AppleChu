use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::memory::PatchMemory;

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
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "bypass AppUser check",
            section: "BypassAppUser",
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
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "bypass 120Hz check",
            section: "Bypass120hz",
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
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "bypass 1080p check",
            section: "Bypass1080p",
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
