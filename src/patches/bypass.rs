use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
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
        &PatchDef {
            name: "绕过 AppUser 检测",
            section: "BypassAppUser",
            pattern: Some("83 7C 24 04 00 75"),
            pattern_offset: 5,
            known_offsets: &[0x89075],
            expected: &[0x75],
            patch: &[0xEB],
        },
    );
}

pub fn apply_bypass_120hz(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "绕过 120Hz 检测",
            section: "Bypass120hz",
            pattern: Some("85 C0 74 3F ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 81 BC 24 34 02 00 00 80 07 00 00"),
            pattern_offset: 0,
            known_offsets: &[0x15810E],
            expected: &[0x85, 0xC0, 0x74, 0x3F],
            patch: &[0xEB, 0x30, 0xEB, 0x2E],
        },
    );
}

fn apply_bypass_1080p(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "绕过 1080P 检测",
            section: "Bypass1080p",
            pattern: Some(
                "81 BC 24 34 02 00 00 80 07 00 00 75 1F 81 BC 24 38 02 00 00 38 04 00 00 75 12",
            ),
            pattern_offset: 0,
            known_offsets: &[0x15811C],
            expected: BYPASS_1080P_EXPECTED,
            patch: BYPASS_1080P_PATCH,
        },
    );
}
