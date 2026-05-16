use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "免费游玩",
            section: "FreePlay",
            pattern: Some("3C 01"),
            pattern_offset: 0,
            known_offsets: &[0x3DF4E4],
            expected: &[0x3C, 0x01],
            patch: &[0x38, 0xC0],
        },
    );
}
