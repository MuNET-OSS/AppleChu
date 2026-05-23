use crate::config::Config;
use crate::util::api::Api;
use crate::util::memory::{file_offset_to_va, patch_bytes, PatchResult};
use crate::util::pattern;

pub struct PatchDef {
    pub name: &'static str,
    pub section: &'static str,
    pub pattern: Option<&'static str>,
    pub pattern_offset: isize,
    pub known_offsets: &'static [u32],
    pub expected: &'static [u8],
    pub patch: &'static [u8],
}

pub fn apply_patch(api: &Api, config: &Config, def: &PatchDef) -> PatchResult {
    if !config.is_enabled(def.section) {
        return PatchResult::AlreadyPatched;
    }

    if let Some(addr) = find_by_pattern(api, def) {
        return log_result(api, def, patch_bytes(api, addr, def.expected, def.patch));
    }

    for offset in def.known_offsets {
        let addr = file_offset_to_va(api, *offset);
        if addr == 0 {
            continue;
        }

        let result = patch_bytes(api, addr, def.expected, def.patch);
        if result != PatchResult::Mismatch {
            return log_result(api, def, result);
        }
    }

    log_result(api, def, PatchResult::Mismatch)
}

fn find_by_pattern(api: &Api, def: &PatchDef) -> Option<usize> {
    let pattern = def.pattern?;
    let found = pattern::scan(api, pattern);
    if found == 0 {
        return None;
    }

    found.checked_add_signed(def.pattern_offset)
}

fn log_result(api: &Api, def: &PatchDef, result: PatchResult) -> PatchResult {
    match result {
        PatchResult::Applied => api.log_info(&format!("patch applied: {}", def.name)),
        PatchResult::AlreadyPatched => api.log_info(&format!("patch already applied: {}", def.name)),
        PatchResult::Mismatch => api.log_warn(&format!("patch bytes mismatch: {}", def.name)),
    }
    result
}
