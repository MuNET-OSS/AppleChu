use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchDef};
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;
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
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "disable encryption 1",
            section: "DisableEncryption",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x17D200C],
            expected: &[0xF5],
            patch: &[0x00],
        },
    );
    apply_patch(
        api,
        config,
        &PatchDef {
            name: "disable encryption 2",
            section: "DisableEncryption",
            pattern: None,
            pattern_offset: 0,
            known_offsets: &[0x17D2010],
            expected: &[0xF5],
            patch: &[0x00],
        },
    );
    apply_disable_tls(api, config);
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
        api.log_info("patch applied: disable TLS (WinHttpOpenRequest IAT hook)");
    } else {
        api.log_warn("disable TLS: WinHttpOpenRequest import not found");
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
