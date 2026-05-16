use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "关闭网络加密 1",
            section: "DisableEncryption",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x17D200C],
            expected: &[0xF5],
            patch: &[0x00],
        },
    );
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "关闭网络加密 2",
            section: "DisableEncryption",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x17D2010],
            expected: &[0xF5],
            patch: &[0x00],
        },
    );
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "关闭 TLS",
            section: "DisableTLS",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0xE0D3FB],
            expected: &[0x80],
            patch: &[0x00],
        },
    );
}
