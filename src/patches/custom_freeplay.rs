use crate::config::Config;
use crate::util::api::Api;
use crate::util::memory::{file_offset_to_va, patch_bytes, write_value, PatchResult};

const FREE_PLAY_TEXT_OFFSET: u32 = 0x14AD9BC;
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

    let length_addr = file_offset_to_va(api, 0x3DF4E9);
    if length_addr == 0 || !write_value(api, length_addr, text_bytes.len() as u8) {
        api.log_warn("补丁写入失败: 自定义 FREE PLAY 文本长度");
        return;
    }

    let text_addr = file_offset_to_va(api, FREE_PLAY_TEXT_OFFSET);
    if text_addr == 0 {
        api.log_warn("自定义 FREE PLAY 文本地址转换失败");
        return;
    }

    match patch_free_play_text(api, text_addr, text_bytes) {
        PatchResult::Applied => api.log_info("补丁已应用: 自定义 FREE PLAY 文本"),
        PatchResult::AlreadyPatched => api.log_info("补丁已存在: 自定义 FREE PLAY 文本"),
        PatchResult::Mismatch => api.log_warn("补丁原始字节不匹配: 自定义 FREE PLAY 文本"),
    }
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
