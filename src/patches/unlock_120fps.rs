use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    if !config.is_enabled("Unlock120fps") {
        return;
    }

    let patch_name = if config.get_bool("Unlock120fps", "force", false) {
        "强制解锁 120fps"
    } else {
        "解锁 120fps"
    };

    // v2.45 的 120fps 解锁与 120Hz 检测绕过共用同一处分支补丁。
    apply_patch(
        api,
        config,
        &PatchDef {
            name: patch_name,
            section: "Unlock120fps",
            pattern: Some("85 C0 74 3F ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? 81 BC 24 34 02 00 00 80 07 00 00"),
            pattern_offset: 0,
            known_offsets: &[0x15810E],
            expected: &[0x85, 0xC0, 0x74, 0x3F],
            patch: &[0xEB, 0x30, 0xEB, 0x2E],
        },
    );
}
