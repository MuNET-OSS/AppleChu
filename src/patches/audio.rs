use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::api::Api;
use crate::util::memory::PatchMemory;

pub fn apply(api: &Api, config: &Config) {
    apply_shared_audio(api, config);
    apply_force_2ch(api, config);
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    apply_force_2ch(api, config);
}

fn apply_force_2ch<M: PatchMemory>(api: &M, config: &Config) {
    apply_patch(
        api,
        config,
        &VersionedPatch {
            name: "force 2ch audio",
            section: "Force2chAudio",
            variants: &[PatchVariant {
                pattern: Some("83 C4 04 85 C0 75 3F 68 ?? ?? ?? ?? E8 ?? ?? ?? ?? B8 02 00 00 00"),
                pattern_offset: 5,
                known_offsets: &[],
                expected: &[0x75, 0x3F],
                patch: &[0x90, 0x90],
            }],
        },
    );
}

fn apply_shared_audio(api: &Api, config: &Config) {
    if !config.is_enabled("ForceSharedAudio") {
        return;
    }

    // MOV dword ptr [ESI+0xa0], 1 → C7 86 A0 00 00 00 01 00 00 00
    // 改 01→00 使 AUDCLNT_SHAREMODE_EXCLUSIVE→SHARED
    let site = api.aob_scan(
        api.text_base(),
        api.text_size(),
        &[0xC7, 0x86, 0xA0, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
        "xxxxxx?xxx",
    );
    if site != 0 {
        let zero = [0u8; 1];
        if api.mem_write(site + 6, &zero) {
            api.log_info("patch applied: force shared audio");
            return;
        }
    }

    api.log_warn("force shared audio: target instruction not found");
}
