use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "跳过启动画面",
            section: "SkipStartup",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x99B21A],
            expected: &[0x6A, 0x01],
            patch: &[0x6A, 0x04],
        },
    );
}
