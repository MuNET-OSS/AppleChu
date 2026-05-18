use std::ffi::c_void;

use crate::hooks::autoplay;
use crate::util::api::Api;

static mut API_HANDLE: Option<Api> = None;
static mut UPSERT_ADDR: usize = 0;
static mut ORIG_UPSERT: usize = 0;

pub fn init(api: &Api) {
    unsafe {
        API_HANDLE = Some(*api);
    }

    let upsert = find_upsert_function(api, api.text_base(), api.text_size(), api.game_base(), api.game_size());
    if upsert == 0 {
        api.log_warn("智能成绩屏蔽初始化失败: UpsertUserAll 未找到，autoplay 时成绩可能上传");
        return;
    }

    let Some(trampoline) = api.hook_create(upsert, hooked_upsert as *const () as usize) else {
        api.log_warn("智能成绩屏蔽初始化失败: UpsertUserAll hook 创建失败");
        return;
    };
    if !api.hook_enable(upsert) {
        api.log_warn("智能成绩屏蔽初始化失败: UpsertUserAll hook 启用失败");
        return;
    }

    unsafe {
        UPSERT_ADDR = upsert;
        ORIG_UPSERT = trampoline;
    }
    api.log_info(&format!(
        "智能成绩屏蔽已启用: UpsertUserAll @ 0x{upsert:08X}，autoplay 开启时成绩不上传"
    ));
}

pub fn shutdown() {
    unsafe {
        if let Some(api) = API_HANDLE {
            if UPSERT_ADDR != 0 {
                api.hook_disable(UPSERT_ADDR);
                api.hook_remove(UPSERT_ADDR);
                UPSERT_ADDR = 0;
            }
            api.log_info("智能成绩屏蔽已清理");
        }
    }
}

unsafe extern "C" fn hooked_upsert(a: *mut c_void, b: *mut c_void, c: *mut c_void) {
    if autoplay::is_enabled() || autoplay::was_used() {
        if let Some(api) = API_HANDLE {
            api.log_info("成绩屏蔽: 本次游玩使用过 autoplay，已阻止上传");
        }
        autoplay::reset_was_used();
        return;
    }

    let orig: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) = std::mem::transmute(ORIG_UPSERT);
    orig(a, b, c);
}

fn find_upsert_function(api: &Api, text_base: usize, text_size: u32, module_base: usize, module_size: u32) -> usize {
    let str_addr = api.aob_scan(
        module_base,
        module_size,
        b"upsertUserAll\0",
        "xxxxxxxxxxxxxx",
    );
    if str_addr == 0 {
        return 0;
    }

    let mut push_pattern = [0u8; 5];
    push_pattern[0] = 0x68;
    push_pattern[1..5].copy_from_slice(&(str_addr as u32).to_le_bytes());
    let push_site = api.aob_scan(text_base, text_size, &push_pattern, "xxxxx");
    if push_site == 0 {
        return 0;
    }

    let mut text = vec![0; text_size as usize];
    if !api.mem_read(text_base, &mut text) {
        return 0;
    }
    let Some(push_offset) = push_site.checked_sub(text_base) else {
        return 0;
    };

    for back in 1..0x200usize {
        let Some(q) = push_offset.checked_sub(back) else {
            break;
        };
        if q + 5 > text.len() {
            continue;
        }
        if text[q..q + 5] == [0x55, 0x8B, 0xEC, 0x6A, 0xFF] {
            let data_builder = text_base + q;
            return find_caller_function(&text, text_base, data_builder).unwrap_or(0);
        }
    }
    0
}

fn find_caller_function(text: &[u8], text_base: usize, data_builder: usize) -> Option<usize> {
    for j in 0..text.len().saturating_sub(5) {
        if text[j] != 0xE8 {
            continue;
        }
        let rel = read_le_i32(text, j + 1)?;
        let call_target = (text_base + j + 5).wrapping_add(rel as usize);
        let matched = call_target == data_builder || jump_target(text, text_base, call_target) == Some(data_builder);
        if !matched {
            continue;
        }

        for back in 1..0x200usize {
            let Some(f) = j.checked_sub(back) else {
                break;
            };
            if f + 5 <= text.len() && text[f..f + 5] == [0x55, 0x8B, 0xEC, 0x6A, 0xFF] {
                return Some(text_base + f);
            }
        }
    }
    None
}

fn jump_target(text: &[u8], text_base: usize, target: usize) -> Option<usize> {
    let offset = target.checked_sub(text_base)?;
    if offset + 5 > text.len() || text[offset] != 0xE9 {
        return None;
    }
    let rel = read_le_i32(text, offset + 1)?;
    Some((target + 5).wrapping_add(rel as usize))
}

fn read_le_i32(buf: &[u8], offset: usize) -> Option<i32> {
    let bytes: [u8; 4] = buf.get(offset..offset + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(bytes))
}
