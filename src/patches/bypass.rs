use crate::config::Config;
use crate::patch_engine::{PatchVariant, VersionedPatch, apply_patch};
use crate::util::api::Api;

const BYPASS_1080P_EXPECTED: &[u8] = &[
    0x81, 0xBC, 0x24, 0x34, 0x02, 0x00, 0x00, 0x80, 0x07, 0x00, 0x00, 0x75, 0x1F, 0x81, 0xBC, 0x24,
    0x38, 0x02, 0x00, 0x00, 0x38, 0x04, 0x00, 0x00, 0x75, 0x12,
];

const BYPASS_1080P_PATCH: &[u8] = &[
    0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
];

pub fn apply(api: &Api, config: &Config) {
    apply_bypass_1080p(api, config);
    apply_bypass_120hz(api, config);
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "bypass AppUser check",
            section: "BypassAppUser",
            variants: &[PatchVariant {
                pattern: Some("83 7C 24 04 00 75"),
                pattern_offset: 5,
                known_offsets: &[],
                expected: &[0x75],
                patch: &[0xEB],
            }],
        },
    );
}

pub fn apply_bypass_120hz(api: &Api, config: &Config) {
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

fn apply_bypass_1080p(api: &Api, config: &Config) {
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
