use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    if !config.is_enabled("Unlock120fps") {
        return;
    }

    // B9 78 00 00 00 B8 3C 00 00 00 0F 45 C1
    // = MOV ECX,120 / MOV EAX,60 / CMOVNZ EAX,ECX
    // 改 MOV EAX,60 → MOV EAX,120 强制 120fps
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "unlock 120fps",
            section: "Unlock120fps",
            pattern: Some("B9 78 00 00 00 B8 3C 00 00 00 0F 45 C1"),
            pattern_offset: 5,
            known_offsets: &[],
            expected: &[0xB8, 0x3C, 0x00, 0x00, 0x00],
            patch: &[0xB8, 0x78, 0x00, 0x00, 0x00],
        },
    );
}
