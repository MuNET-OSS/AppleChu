use crate::config::Config;
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;
use crate::util::pattern;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

const WINHTTP_DLL: &str = "winhttp.dll";
const WINHTTP_OPEN_REQUEST: &str = "WinHttpOpenRequest";
// WINHTTP_FLAG_SECURE = 0x00800000
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;

type WinHttpOpenRequestFn = unsafe extern "system" fn(
    *mut c_void, *const u16, *const u16, *const u16,
    *const u16, *const *const u16, u32,
) -> *mut c_void;

static ORIG_OPEN_REQUEST: AtomicUsize = AtomicUsize::new(0);

pub fn apply(api: &Api, config: &Config) {
    apply_disable_encryption(api, config);
    apply_disable_tls(api, config);
}

fn apply_disable_encryption(api: &Api, config: &Config) {
    if !config.is_enabled("DisableEncryption") {
        return;
    }

    let found = pattern::scan_bytes(api, b"cannot encrypt.\0");
    if found == 0 {
        api.log_warn("关闭网络加密: 未找到加密标识字符串");
        return;
    }

    let addr_bytes = (found as u32).to_le_bytes();
    // 68 [addr] = PUSH <string_addr>
    let mut push_sig = [0u8; 5];
    push_sig[0] = 0x68;
    push_sig[1..5].copy_from_slice(&addr_bytes);

    let text_base = api.text_base();
    let text_size = api.text_size();
    let mut search_start = text_base;
    let mut patched = 0u32;

    loop {
        let remaining = text_size.saturating_sub((search_start - text_base) as u32);
        if remaining < 5 {
            break;
        }

        let push_site = api.aob_scan(search_start, remaining, &push_sig, "xxxxx");
        if push_site == 0 {
            break;
        }

        if let Some(func_start) = find_function_start(api, push_site, text_base) {
            if patch_encrypt_flag_in_function(api, func_start, push_site) {
                patched += 1;
            }
        }

        search_start = push_site + 5;
    }

    if patched > 0 {
        api.log_info(&format!("补丁已应用: 关闭网络加密 ({patched} 处)"));
    } else {
        api.log_warn("关闭网络加密: 未找到加密标志");
    }
}

fn find_function_start(api: &Api, addr: usize, text_base: usize) -> Option<usize> {
    // 55 8B EC 6A FF = PUSH EBP / MOV EBP,ESP / PUSH -1
    let prologue = [0x55, 0x8B, 0xEC, 0x6A, 0xFF];
    for back in 1..0x800usize {
        let candidate = addr.checked_sub(back)?;
        if candidate < text_base {
            return None;
        }
        let mut buf = [0u8; 5];
        if api.mem_read(candidate, &mut buf) && buf == prologue {
            return Some(candidate);
        }
    }
    None
}

fn patch_encrypt_flag_in_function(api: &Api, func_start: usize, ref_site: usize) -> bool {
    let func_end = ref_site + 0x200;
    // MOV dword ptr [param_1+4], imm32 → C7 41 04 xx xx xx xx
    let mut scan_addr = func_start;
    while scan_addr < func_end {
        let remaining = (func_end - scan_addr) as u32;
        if remaining < 7 {
            break;
        }
        let site = api.aob_scan(scan_addr, remaining, &[0xC7, 0x41, 0x04], "xxx");
        if site == 0 {
            break;
        }
        let mut val_buf = [0u8; 4];
        if api.mem_read(site + 3, &mut val_buf) {
            let val = u32::from_le_bytes(val_buf);
            if val != 0 && val < 0x1000 {
                let zero = [0u8; 4];
                if api.mem_write(site + 3, &zero) {
                    return true;
                }
            }
        }
        scan_addr = site + 7;
    }
    false
}

fn apply_disable_tls(api: &Api, config: &Config) {
    if !config.is_enabled("DisableTLS") {
        return;
    }

    let original = unsafe {
        hook_iat(
            api.game_base(),
            WINHTTP_DLL,
            WINHTTP_OPEN_REQUEST,
            hooked_open_request as *const (),
        )
    };

    if let Some(orig) = original {
        ORIG_OPEN_REQUEST.store(orig as usize, Ordering::SeqCst);
        api.log_info("补丁已应用: 关闭 TLS (WinHttpOpenRequest IAT hook)");
    } else {
        api.log_warn("关闭 TLS: 未找到 WinHttpOpenRequest 导入");
    }
}

unsafe extern "system" fn hooked_open_request(
    h_connect: *mut c_void,
    verb: *const u16,
    object_name: *const u16,
    version: *const u16,
    referrer: *const u16,
    accept_types: *const *const u16,
    flags: u32,
) -> *mut c_void {
    let orig_addr = ORIG_OPEN_REQUEST.load(Ordering::SeqCst);
    if orig_addr == 0 {
        return std::ptr::null_mut();
    }

    let orig: WinHttpOpenRequestFn = std::mem::transmute(orig_addr);
    orig(h_connect, verb, object_name, version, referrer, accept_types, flags & !WINHTTP_FLAG_SECURE)
}
