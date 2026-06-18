use crate::config::Config;
use crate::util::api::Api;
use crate::util::memory::{PatchResult, patch_bytes};
use crate::util::pattern;

const FORCE_TRUE_CALLSITE: &str = "E8 ? ? ? ? 84 C0 74 DB 8D 8B ? 00 00 00 E8";
const SKIP_FAILURE_BRANCH: &str = "C2 83 F8 07 74 20";

pub fn apply(api: &Api, config: &Config) {
    if !config.is_enabled("D3D9Ex")
        || !config.get_bool("D3D9Ex", "enable", true)
        || !config.get_bool("D3D9Ex", "fast_restart", true)
    {
        return;
    }

    log_result(api, "fast restart force-true", patch_force_true(api));
    log_result(api, "fast restart branch skip", patch_branch_skip(api));
}

fn patch_force_true(api: &Api) -> PatchResult {
    let callsite = pattern::scan(api, FORCE_TRUE_CALLSITE);
    if callsite == 0 {
        return PatchResult::Mismatch;
    }

    let mut rel = [0u8; 4];
    if !api.mem_read(callsite + 1, &mut rel) {
        return PatchResult::Mismatch;
    }

    let rel = i32::from_le_bytes(rel);
    let Some(target) = (callsite + 5).checked_add_signed(rel as isize) else {
        return PatchResult::Mismatch;
    };

    patch_already_or_write(api, target, &[0xB0, 0x01, 0xC3])
}

fn patch_branch_skip(api: &Api) -> PatchResult {
    let found = pattern::scan(api, SKIP_FAILURE_BRANCH);
    if found == 0 {
        return PatchResult::Mismatch;
    }

    patch_bytes(api, found + 4, &[0x74], &[0xEB])
}

fn patch_already_or_write(api: &Api, addr: usize, patch: &[u8]) -> PatchResult {
    let mut current = vec![0; patch.len()];
    if !api.mem_read(addr, &mut current) {
        return PatchResult::Mismatch;
    }
    if current == patch {
        return PatchResult::AlreadyPatched;
    }
    if api.mem_write(addr, patch) {
        PatchResult::Applied
    } else {
        PatchResult::Mismatch
    }
}

fn log_result(api: &Api, name: &str, result: PatchResult) {
    match result {
        PatchResult::Applied => api.log_info(&format!("patch applied: {name}")),
        PatchResult::AlreadyPatched => api.log_info(&format!("patch already applied: {name}")),
        PatchResult::Mismatch => api.log_warn(&format!("patch bytes mismatch: {name}")),
    }
}
