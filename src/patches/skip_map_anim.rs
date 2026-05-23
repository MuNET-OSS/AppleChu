use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "skip map animation",
            section: "SkipMapAnimation",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x8D63DA],
            expected: &[0x01],
            patch: &[0x00],
        },
    );
}
