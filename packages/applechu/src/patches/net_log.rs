use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

const WINHTTP_DLL: &str = "winhttp.dll";

type WinHttpConnectFn = unsafe extern "system" fn(*mut c_void, *const u16, u16, u32) -> *mut c_void;
type WinHttpOpenRequestFn = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    *const u16,
    *const u16,
    *const u16,
    *const *const u16,
    u32,
) -> *mut c_void;
type WinHttpSendRequestFn =
    unsafe extern "system" fn(*mut c_void, *const u16, u32, *const c_void, u32, u32, usize) -> i32;
type WinHttpReceiveResponseFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type WinHttpQueryHeadersFn =
    unsafe extern "system" fn(*mut c_void, u32, *const u16, *mut c_void, *mut u32, *mut u32) -> i32;

const WINHTTP_QUERY_STATUS_CODE: u32 = 19;
const WINHTTP_QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

static ORIG_CONNECT: AtomicUsize = AtomicUsize::new(0);
static ORIG_OPEN_REQUEST: AtomicUsize = AtomicUsize::new(0);
static ORIG_SEND_REQUEST: AtomicUsize = AtomicUsize::new(0);
static ORIG_RECEIVE_RESPONSE: AtomicUsize = AtomicUsize::new(0);
static ORIG_QUERY_HEADERS: AtomicUsize = AtomicUsize::new(0);

crate::config_section! {
    pub(crate) struct NetLogConfig => NET_LOG_CONFIG_SECTION {
        section: "NetLog",
        order: 175,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "network",
        comment: "网络请求日志",
        fields: {}
    }
}

pub fn install_pre_entry_hook(api: &Api, config: &Config) {
    if !config
        .section::<NetLogConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }

    unsafe {
        hook(
            api,
            "WinHttpConnect",
            hooked_connect as *const (),
            &ORIG_CONNECT,
        );
        hook(
            api,
            "WinHttpOpenRequest",
            hooked_open_request as *const (),
            &ORIG_OPEN_REQUEST,
        );
        hook(
            api,
            "WinHttpSendRequest",
            hooked_send_request as *const (),
            &ORIG_SEND_REQUEST,
        );
        hook(
            api,
            "WinHttpReceiveResponse",
            hooked_receive_response as *const (),
            &ORIG_RECEIVE_RESPONSE,
        );
        hook(
            api,
            "WinHttpQueryHeaders",
            hooked_query_headers as *const (),
            &ORIG_QUERY_HEADERS,
        );
    }

    api.log_info("patch applied: network request logging (WinHTTP)");
}

unsafe fn hook(api: &Api, name: &str, detour: *const (), slot: &AtomicUsize) {
    if let Some(orig) = hook_iat(api.game_base(), WINHTTP_DLL, name, detour) {
        slot.store(orig as usize, Ordering::SeqCst);
    } else {
        api.log_warn(&format!("net_log: {name} import not found"));
    }
}

unsafe extern "system" fn hooked_connect(
    session: *mut c_void,
    server_name: *const u16,
    port: u16,
    reserved: u32,
) -> *mut c_void {
    let orig: WinHttpConnectFn = std::mem::transmute(ORIG_CONNECT.load(Ordering::SeqCst));
    let result = orig(session, server_name, port, reserved);
    let host = wide(server_name);
    if result.is_null() {
        log_error(&format!(
            "[NET] Connect FAILED host={host} port={port} err={}",
            last_error()
        ));
    } else {
        log_info(&format!("[NET] Connect host={host} port={port}"));
    }
    result
}

unsafe extern "system" fn hooked_open_request(
    connect: *mut c_void,
    verb: *const u16,
    object_name: *const u16,
    version: *const u16,
    referrer: *const u16,
    accept_types: *const *const u16,
    flags: u32,
) -> *mut c_void {
    let orig: WinHttpOpenRequestFn = std::mem::transmute(ORIG_OPEN_REQUEST.load(Ordering::SeqCst));
    let result = orig(
        connect,
        verb,
        object_name,
        version,
        referrer,
        accept_types,
        flags,
    );
    let method = wide(verb);
    let path = wide(object_name);
    if result.is_null() {
        log_error(&format!(
            "[NET] OpenRequest FAILED {method} {path} flags={flags:#x} err={}",
            last_error()
        ));
    } else {
        log_info(&format!(
            "[NET] OpenRequest {method} {path} (flags={flags:#x})"
        ));
    }
    result
}

unsafe extern "system" fn hooked_send_request(
    request: *mut c_void,
    headers: *const u16,
    headers_len: u32,
    optional: *const c_void,
    optional_len: u32,
    total_len: u32,
    context: usize,
) -> i32 {
    let orig: WinHttpSendRequestFn = std::mem::transmute(ORIG_SEND_REQUEST.load(Ordering::SeqCst));
    let result = orig(
        request,
        headers,
        headers_len,
        optional,
        optional_len,
        total_len,
        context,
    );
    if result == 0 {
        log_error(&format!("[NET] SendRequest FAILED err={}", last_error()));
    }
    result
}

unsafe extern "system" fn hooked_receive_response(
    request: *mut c_void,
    reserved: *mut c_void,
) -> i32 {
    let orig: WinHttpReceiveResponseFn =
        std::mem::transmute(ORIG_RECEIVE_RESPONSE.load(Ordering::SeqCst));
    let result = orig(request, reserved);
    if result == 0 {
        log_error(&format!(
            "[NET] ReceiveResponse FAILED err={}",
            last_error()
        ));
    }
    result
}

unsafe extern "system" fn hooked_query_headers(
    request: *mut c_void,
    info_level: u32,
    name: *const u16,
    buffer: *mut c_void,
    buffer_len: *mut u32,
    index: *mut u32,
) -> i32 {
    let orig: WinHttpQueryHeadersFn =
        std::mem::transmute(ORIG_QUERY_HEADERS.load(Ordering::SeqCst));
    let result = orig(request, info_level, name, buffer, buffer_len, index);
    if result != 0
        && info_level == (WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER)
        && !buffer.is_null()
    {
        let status = *(buffer as *const u32);
        match status {
            500..=599 => log_error(&format!("[NET] HTTP status={status}")),
            400..=499 => log_warn(&format!("[NET] HTTP status={status}")),
            _ => log_info(&format!("[NET] HTTP status={status}")),
        }
    }
    result
}

fn log_info(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}

fn log_warn(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_warn(message);
    }
}

fn log_error(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_error(message);
    }
}

unsafe fn wide(ptr: *const u16) -> String {
    if ptr.is_null() {
        return "<null>".to_owned();
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

unsafe fn last_error() -> u32 {
    GetLastError()
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
}
