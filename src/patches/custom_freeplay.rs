use crate::config::Config;
use crate::util::api::Api;
use crate::util::memory::{patch_bytes, write_value, PatchResult};
use crate::util::pattern;

const FREE_PLAY_TEXT_EXPECTED: &[u8] = b"FREE PLAY";
const MAX_CUSTOM_TEXT_LEN: usize = u8::MAX as usize;

pub fn apply(api: &Api, config: &Config) {
    if !config.is_enabled("FreePlay") {
        return;
    }

    let custom_text = config.get_string("FreePlay", "custom_text", "");
    if custom_text.is_empty() {
        return;
    }

    let text_bytes = custom_text.as_bytes();
    if text_bytes.len() > MAX_CUSTOM_TEXT_LEN {
        api.log_warn("自定义 FREE PLAY 文本过长，已跳过");
        return;
    }

    let text_addr = pattern::scan_bytes(api, b"FREE PLAY\0");
    if text_addr == 0 {
        api.log_warn("自定义 FREE PLAY 文本: 未找到字符串");
        return;
    }

    let length_addr = find_length_addr(api, text_addr);
    if length_addr == 0 || !write_value(api, length_addr, text_bytes.len() as u8) {
        api.log_warn("补丁写入失败: 自定义 FREE PLAY 文本长度");
        return;
    }

    match patch_free_play_text(api, text_addr, text_bytes) {
        PatchResult::Applied => api.log_info("补丁已应用: 自定义 FREE PLAY 文本"),
        PatchResult::AlreadyPatched => api.log_info("补丁已存在: 自定义 FREE PLAY 文本"),
        PatchResult::Mismatch => api.log_warn("补丁原始字节不匹配: 自定义 FREE PLAY 文本"),
    }
}

fn find_length_addr(api: &Api, text_addr: usize) -> usize {
    // PUSH len / PUSH text_addr → 6A xx 68 [addr32_LE]
    let addr_bytes = (text_addr as u32).to_le_bytes();
    let mut sig = [0u8; 7];
    sig[0] = 0x6A; // PUSH imm8
    // sig[1] = length (wildcard)
    sig[2] = 0x68; // PUSH imm32
    sig[3..7].copy_from_slice(&addr_bytes);
    let mask = "x?xxxxx";
    let found = api.aob_scan(api.text_base(), api.text_size(), &sig, mask);
    if found == 0 {
        return 0;
    }
    // length operand is at found + 1
    found + 1
}

fn patch_free_play_text(api: &Api, addr: usize, text: &[u8]) -> PatchResult {
    if text.len() == FREE_PLAY_TEXT_EXPECTED.len() {
        return patch_bytes(api, addr, FREE_PLAY_TEXT_EXPECTED, text);
    }

    let mut current = vec![0; FREE_PLAY_TEXT_EXPECTED.len()];
    if !api.mem_read(addr, &mut current) {
        return PatchResult::Mismatch;
    }

    if current != FREE_PLAY_TEXT_EXPECTED && !is_zero_padded_text(api, addr, text) {
        return PatchResult::Mismatch;
    }

    let mut buffer = Vec::from(text);
    buffer.push(0);
    if api.mem_write(addr, &buffer) {
        PatchResult::Applied
    } else {
        PatchResult::Mismatch
    }
}

fn is_zero_padded_text(api: &Api, addr: usize, text: &[u8]) -> bool {
    let mut current = vec![0; text.len().saturating_add(1)];
    api.mem_read(addr, &mut current)
        && current.starts_with(text)
        && current.get(text.len()) == Some(&0)
}
