use crate::util::memory::{file_offset_to_va, patch_bytes, PatchMemory, PatchResult};
use crate::util::pattern;

pub struct VersionedPatch {
    pub name: &'static str,
    pub variants: &'static [PatchVariant],
}

pub struct PatchVariant {
    pub pattern: Option<&'static str>,
    pub pattern_offset: isize,
    pub known_offsets: &'static [u32],
    pub expected: &'static [u8],
    pub patch: &'static [u8],
}

pub fn apply_patch<M: PatchMemory>(api: &M, def: &VersionedPatch) -> PatchResult {
    let mut fallback = PatchResult::Mismatch;
    for variant in def.variants {
        if let Some(addr) = find_by_pattern(api, variant) {
            let result = patch_bytes(api, addr, variant.expected, variant.patch);
            if result != PatchResult::Mismatch {
                return log_result(api, def, result);
            }
        }

        for offset in variant.known_offsets {
            let addr = file_offset_to_va(api, *offset);
            if addr == 0 {
                continue;
            }

            let result = patch_bytes(api, addr, variant.expected, variant.patch);
            if let Some(decisive) = classify_known_offset_result(result, &mut fallback) {
                return log_result(api, def, decisive);
            }
        }
    }

    log_result(api, def, fallback)
}

fn find_by_pattern<M: PatchMemory>(api: &M, variant: &PatchVariant) -> Option<usize> {
    let pattern = variant.pattern?;
    let found = pattern::scan(api, pattern);
    if found == 0 {
        return None;
    }

    found.checked_add_signed(variant.pattern_offset)
}

fn classify_known_offset_result(
    result: PatchResult,
    fallback: &mut PatchResult,
) -> Option<PatchResult> {
    match result {
        PatchResult::Applied => Some(PatchResult::Applied),
        PatchResult::AlreadyPatched => {
            *fallback = PatchResult::AlreadyPatched;
            None
        }
        PatchResult::Mismatch => None,
    }
}

fn log_result<M: PatchMemory>(api: &M, def: &VersionedPatch, result: PatchResult) -> PatchResult {
    match result {
        PatchResult::Applied => api.log_info(&format!("Patch applied: {}", def.name)),
        PatchResult::AlreadyPatched => {
            api.log_info(&format!("Patch already applied: {}", def.name))
        }
        PatchResult::Mismatch => api.log_warn(&format!("Patch signature mismatch: {}", def.name)),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_offset_already_patched_is_deferred_for_later_variant() {
        // Given: no earlier patch attempt has produced a usable result.
        let mut fallback = PatchResult::Mismatch;

        // When: a version-specific known offset happens to contain the patch byte.
        let decisive = classify_known_offset_result(PatchResult::AlreadyPatched, &mut fallback);

        // Then: the result is remembered but later variants must still be tried.
        assert_eq!(decisive, None);
        assert_eq!(fallback, PatchResult::AlreadyPatched);
    }
}
