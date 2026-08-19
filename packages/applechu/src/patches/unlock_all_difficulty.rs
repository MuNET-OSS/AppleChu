use crate::config::Config;
use crate::util::memory::{patch_bytes, PatchMemory, PatchResult};
use crate::util::pattern;

const CALL_OFFSET: usize = 6;
const MAIN_SIGNATURE: &str = concat!(
    "38 44 24 13 74 09 E8 ?? ?? ?? ?? 3C 01 75 1C ",
    "6A 01 8D 44 24 14 8B CF 50 E8 ?? ?? ?? ?? ",
    "6A 01 8D 44 24 14 8B CF 50 E8"
);
const LEGACY_SIGNATURE: &str = concat!(
    "38 44 24 13 74 09 E8 ?? ?? ?? ?? 3C 01 75 1C ",
    "6A 01 8D 44 24 14 50 8B CF E8 ?? ?? ?? ?? ",
    "6A 01 8D 44 24 14 50 8B CF E8"
);

crate::config_section! {
    pub(crate) struct UnlockAllDifficultyConfig => UNLOCK_ALL_DIFFICULTY_CONFIG_SECTION {
        section: "UnlockAllDifficulty",
        order: 185,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "gameplay",
        comment: "直接解锁全部难度",
        fields: {}
    }
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    if !config
        .section::<UnlockAllDifficultyConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }

    for signature in [MAIN_SIGNATURE, LEGACY_SIGNATURE] {
        let Some(hit) = find_unique(api, signature) else {
            continue;
        };
        let Some(target) = resolve_call_target(api, hit + CALL_OFFSET) else {
            continue;
        };

        match patch_bytes(api, target, &[0x32, 0xC0, 0xC3], &[0xB0, 0x01, 0xC3]) {
            PatchResult::Applied => {
                api.log_info("Patch applied: unlock all difficulties");
                return;
            }
            PatchResult::AlreadyPatched => {
                api.log_info("Patch already applied: unlock all difficulties");
                return;
            }
            PatchResult::Mismatch => {}
        }
    }

    api.log_warn("Patch signature mismatch: unlock all difficulties");
}

fn find_unique<M: PatchMemory>(api: &M, signature: &str) -> Option<usize> {
    let first = pattern::scan(api, signature);
    if first == 0 {
        return None;
    }

    let image_end = api.game_base().checked_add(api.game_size() as usize)?;
    let next = first.checked_add(1)?;
    if next < image_end && pattern::scan_range(api, next, (image_end - next) as u32, signature) != 0
    {
        api.log_warn("Unlock all difficulties signature is ambiguous");
        return None;
    }

    Some(first)
}

fn resolve_call_target<M: PatchMemory>(api: &M, call: usize) -> Option<usize> {
    let mut target = resolve_relative(api, call, 0xE8)?;
    if !address_in_image(api, target, 1) {
        return None;
    }
    let mut opcode = [0_u8; 1];
    if !api.mem_read(target, &mut opcode) {
        return None;
    }
    if opcode[0] == 0xE9 {
        target = resolve_relative(api, target, 0xE9)?;
    }

    address_in_image(api, target, 3).then_some(target)
}

fn resolve_relative<M: PatchMemory>(api: &M, instruction: usize, opcode: u8) -> Option<usize> {
    let mut bytes = [0_u8; 5];
    if !api.mem_read(instruction, &mut bytes) || bytes[0] != opcode {
        return None;
    }

    let displacement = i32::from_le_bytes(bytes[1..5].try_into().ok()?);
    instruction
        .checked_add(5)?
        .checked_add_signed(displacement as isize)
}

fn address_in_image<M: PatchMemory>(api: &M, address: usize, size: usize) -> bool {
    let start = api.game_base();
    let Some(end) = start.checked_add(api.game_size() as usize) else {
        return false;
    };
    address >= start && address.checked_add(size).is_some_and(|last| last <= end)
}
