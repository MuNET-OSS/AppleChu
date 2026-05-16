use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;

pub fn apply(api: &Api, config: &Config) {
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "强制共享音频",
            section: "ForceSharedAudio",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0xE29393],
            expected: &[0x01],
            patch: &[0x00],
        },
    );
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "强制双声道输出",
            section: "Force2chAudio",
            pattern: Some("75 3F"),
            pattern_offset: 0,
            known_offsets: &[0xE2944B],
            expected: &[0x75, 0x3F],
            patch: &[0x90, 0x90],
        },
    );
}
