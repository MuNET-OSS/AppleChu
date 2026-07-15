use crate::config::Config;
use crate::util::memory::PatchMemory;
use crate::util::pattern;

const X_VERSE_PATTERN: &str = "58 2D 56 45 52 53 45";
const MAX_VERSION_TEXT_LEN: usize = 64;

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    let version_text = config.get_string("General", "version_text", "");
    if version_text.is_empty() {
        return;
    }

    let bytes = version_text.as_bytes();
    if bytes.len() >= MAX_VERSION_TEXT_LEN {
        api.log_warn("custom version text too long, skipped");
        return;
    }

    let pattern_addr = pattern::scan_range(api, api.game_base(), api.game_size(), X_VERSE_PATTERN);
    if pattern_addr == 0 {
        api.log_warn("version text pattern not found: X-VERSE");
        return;
    }

    let Some(start_addr) = find_string_start(api, pattern_addr) else {
        api.log_warn("version text start position not found");
        return;
    };

    let mut buffer = [0_u8; MAX_VERSION_TEXT_LEN];
    buffer[..bytes.len()].copy_from_slice(bytes);
    if api.mem_write(start_addr, &buffer) {
        api.log_info("patch applied: custom version text");
    } else {
        api.log_warn("patch write failed: custom version text");
    }
}

fn find_string_start<M: PatchMemory>(api: &M, pattern_addr: usize) -> Option<usize> {
    let search_start = pattern_addr.saturating_sub(MAX_VERSION_TEXT_LEN);
    let search_len = pattern_addr.checked_sub(search_start)?;
    let mut bytes = vec![0; search_len];
    if !api.mem_read(search_start, &mut bytes) {
        return None;
    }

    bytes
        .iter()
        .rposition(|byte| *byte == 0)
        .map(|zero_pos| search_start + zero_pos + 1)
        .filter(|start| *start < pattern_addr)
}
