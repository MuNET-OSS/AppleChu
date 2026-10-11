//! 网络响应桥
//!
//! 在游戏自身的 WinHTTP 调用上挂钩，取出**完整响应体**与**请求路径**，
//! 再广播给订阅者。对应 AquaMai 的 `NetPacketHook.OnNetPacketComplete`。
//!
//! 之所以在这一层而不是原生分发器（RVA 0x59FA60）上挂钩：分发器依赖固定
//! RVA，跨版本会漂移；WinHTTP 导入是稳定 ABI，且 `patches/network.rs` 的
//! `DisableEncryption` 默认生效时响应本就是明文，无需调用 `gm::Aes` 解密。
//!
//! 逻辑 API 名从请求路径末段取得（如 `.../GetGameSettingApi` → `GetGameSettingApi`），
//! 与游戏自身的 API 名表（宽字符串 `0x18F46BC` 起）一致。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::Config;
use crate::server_resources::json::{self, JsonValue};
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

const WINHTTP_DLL: &str = "winhttp.dll";
const WINHTTP_OPEN_REQUEST: &str = "WinHttpOpenRequest";
const WINHTTP_QUERY_DATA_AVAILABLE: &str = "WinHttpQueryDataAvailable";
const WINHTTP_READ_DATA: &str = "WinHttpReadData";
const WINHTTP_CLOSE_HANDLE: &str = "WinHttpCloseHandle";

type WinHttpOpenRequestFn = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    *const u16,
    *const u16,
    *const u16,
    *const *const u16,
    u32,
) -> *mut c_void;
type WinHttpQueryDataAvailableFn = unsafe extern "system" fn(*mut c_void, *mut u32) -> i32;
type WinHttpReadDataFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut u32) -> i32;
type WinHttpCloseHandleFn = unsafe extern "system" fn(*mut c_void) -> i32;

static ORIG_OPEN_REQUEST: AtomicUsize = AtomicUsize::new(0);
static ORIG_QUERY_DATA_AVAILABLE: AtomicUsize = AtomicUsize::new(0);
static ORIG_READ_DATA: AtomicUsize = AtomicUsize::new(0);
static ORIG_CLOSE_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// 响应处理回调：返回 `true` 表示已改写响应，需要重新序列化回写
pub type ResponseHandler = fn(api: &str, response: &mut JsonValue) -> bool;

static HANDLERS: OnceLock<Mutex<Vec<ResponseHandler>>> = OnceLock::new();
static REQUESTS: OnceLock<Mutex<HashMap<usize, RequestState>>> = OnceLock::new();

/// 单个请求的累积状态
#[derive(Default)]
struct RequestState {
    /// 请求路径（用于解析逻辑 API 名）
    path: String,
    /// 已读取的响应体
    body: Vec<u8>,
    /// 是否已派发，避免同一请求重复处理
    dispatched: bool,
}

fn handlers() -> &'static Mutex<Vec<ResponseHandler>> {
    HANDLERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn requests() -> &'static Mutex<HashMap<usize, RequestState>> {
    REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 注册响应处理回调
pub fn subscribe(handler: ResponseHandler) {
    if let Ok(mut handlers) = handlers().lock() {
        if !handlers.contains(&handler) {
            handlers.push(handler);
        }
    }
}

/// 安装 WinHTTP 响应桥
pub fn install(api: &Api, config: &Config) -> Result<(), String> {
    if !crate::server_resources::is_enabled(config) {
        return Ok(());
    }

    unsafe {
        let opened = hook_iat(
            api.game_base(),
            WINHTTP_DLL,
            WINHTTP_OPEN_REQUEST,
            hooked_open_request as *const (),
        );
        let Some(opened) = opened else {
            return Err("WinHttpOpenRequest import not found".to_owned());
        };
        ORIG_OPEN_REQUEST.store(opened as usize, Ordering::SeqCst);

        let queried = hook_iat(
            api.game_base(),
            WINHTTP_DLL,
            WINHTTP_QUERY_DATA_AVAILABLE,
            hooked_query_data_available as *const (),
        );
        let Some(queried) = queried else {
            return Err("WinHttpQueryDataAvailable import not found".to_owned());
        };
        ORIG_QUERY_DATA_AVAILABLE.store(queried as usize, Ordering::SeqCst);

        let read = hook_iat(
            api.game_base(),
            WINHTTP_DLL,
            WINHTTP_READ_DATA,
            hooked_read_data as *const (),
        );
        let Some(read) = read else {
            return Err("WinHttpReadData import not found".to_owned());
        };
        ORIG_READ_DATA.store(read as usize, Ordering::SeqCst);

        let closed = hook_iat(
            api.game_base(),
            WINHTTP_DLL,
            WINHTTP_CLOSE_HANDLE,
            hooked_close_handle as *const (),
        );
        if let Some(closed) = closed {
            ORIG_CLOSE_HANDLE.store(closed as usize, Ordering::SeqCst);
        }
    }

    api.log_info("patch applied: net response bridge (WinHTTP)");
    Ok(())
}

/// 上传一次完整响应：解析 JSON 后依次交给订阅者
///
/// 响应不是合法 JSON 时静默跳过（游戏存在二进制/空响应）。
fn dispatch(path: &str, body: &[u8]) {
    let Some(name) = api_name_from_path(path) else {
        return;
    };
    let Ok(text) = std::str::from_utf8(body) else {
        return;
    };
    // 只处理 JSON 对象响应；纯文本/二进制响应不进入改写流程
    let Some(mut value) = json::parse(text.trim_start_matches('\u{feff}')) else {
        return;
    };

    let mut modified = false;
    if let Ok(handlers) = handlers().lock() {
        for handler in handlers.iter() {
            if handler(&name, &mut value) {
                modified = true;
            }
        }
    }

    // 同时广播给 chu-abi 模组，使其能读取/协作处理该响应。
    // 载荷为 `{"api":"<名>","response":<原始响应>}`，便于订阅方按 API 过滤。
    if let Some(api) = crate::util::api::API.get() {
        let mut envelope = String::with_capacity(value.to_string().len() + 32);
        envelope.push_str("{\"api\":");
        write_json_string(&name, &mut envelope);
        envelope.push_str(",\"response\":");
        envelope.push_str(&value.to_string());
        envelope.push('}');
        api.publish(PUBLISH_TOPIC, envelope.as_bytes());
    }

    // 改写结果记录到日志；回写由调用方（游戏自身缓冲）决定，
    // 本桥只读响应，不修改游戏的内存流。
    if modified {
        log_info(&format!(
            "net response modified by subscriber: {name} ({} bytes)",
            value.to_string().len()
        ));
    }
}

/// 对外广播主题名（chu-abi 模组按此订阅）
const PUBLISH_TOPIC: &str = "net_packet";

/// 在本模块内构造转义后的 JSON 字符串（避免把 json 模块的私有工具外泄）
fn write_json_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            control if (control as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04x}", control as u32);
            }
            other => output.push(other),
        }
    }
    output.push('"');
}

/// 从请求路径解析逻辑 API 名（末段，去掉查询串与扩展）
fn api_name_from_path(path: &str) -> Option<String> {
    let without_query = path.split(['?', '#']).next().unwrap_or(path);
    let segment = without_query
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())?;
    (!segment.is_empty()).then(|| segment.to_owned())
}

unsafe fn with_request<F: FnOnce(&mut RequestState)>(handle: usize, modify: F) {
    if let Ok(mut requests) = requests().lock() {
        let state = requests.entry(handle).or_default();
        modify(state);
    }
}

/// 完成一次派发：取出累积的响应体，标记已派发
unsafe fn finish_request(handle: usize) {
    let payload = {
        let Ok(mut requests) = requests().lock() else {
            return;
        };
        match requests.get_mut(&handle) {
            Some(state) if !state.dispatched && !state.body.is_empty() => {
                state.dispatched = true;
                let body = std::mem::take(&mut state.body);
                Some((state.path.clone(), body))
            }
            _ => None,
        }
    };
    if let Some((path, body)) = payload {
        dispatch(&path, &body);
    }
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
    let original = ORIG_OPEN_REQUEST.load(Ordering::SeqCst);
    if original == 0 {
        return std::ptr::null_mut();
    }
    let original: WinHttpOpenRequestFn = std::mem::transmute(original);
    let result = original(
        connect,
        verb,
        object_name,
        version,
        referrer,
        accept_types,
        flags,
    );

    if !result.is_null() && !object_name.is_null() {
        let path = crate::platform::winapi::wide_to_string(object_name).unwrap_or_default();
        with_request(result as usize, |state| {
            state.path = path;
            state.body.clear();
            state.dispatched = false;
        });
    }
    result
}

unsafe extern "system" fn hooked_query_data_available(
    request: *mut c_void,
    available: *mut u32,
) -> i32 {
    let original = ORIG_QUERY_DATA_AVAILABLE.load(Ordering::SeqCst);
    if original == 0 {
        return 0;
    }
    let original: WinHttpQueryDataAvailableFn = std::mem::transmute(original);
    let result = original(request, available);
    if result != 0 && !available.is_null() && *available == 0 {
        // 可用字节为 0 表示响应体结束，此时游戏不会再调用 ReadData
        finish_request(request as usize);
    }
    result
}

unsafe extern "system" fn hooked_read_data(
    request: *mut c_void,
    buffer: *mut c_void,
    bytes_to_read: u32,
    bytes_read: *mut u32,
) -> i32 {
    let original = ORIG_READ_DATA.load(Ordering::SeqCst);
    if original == 0 {
        return 0;
    }
    let original: WinHttpReadDataFn = std::mem::transmute(original);
    let result = original(request, buffer, bytes_to_read, bytes_read);
    if result != 0 && !bytes_read.is_null() {
        let read = *bytes_read as usize;
        if read > 0 && !buffer.is_null() {
            let chunk = std::slice::from_raw_parts(buffer as *const u8, read);
            with_request(request as usize, |state| {
                // 上限保护：异常响应不至于无界增长
                const MAX_BODY: usize = 8 * 1024 * 1024;
                if state.body.len() + chunk.len() <= MAX_BODY {
                    state.body.extend_from_slice(chunk);
                }
            });
        } else if read == 0 {
            // 读到 0 字节表示响应体结束
            finish_request(request as usize);
        }
    }
    result
}

unsafe extern "system" fn hooked_close_handle(handle: *mut c_void) -> i32 {
    let original = ORIG_CLOSE_HANDLE.load(Ordering::SeqCst);
    if original == 0 {
        return 0;
    }
    // 游戏可能不等待 0 字节就读完即关闭，这里兜底派发一次
    finish_request(handle as usize);
    if let Ok(mut requests) = requests().lock() {
        requests.remove(&(handle as usize));
    }
    let original: WinHttpCloseHandleFn = std::mem::transmute(original);
    original(handle)
}

fn log_info(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_name_is_last_path_segment() {
        // Given: 游戏实际使用的几种请求路径形态。
        let cases = [
            ("/NetDelivery/GetGameSettingApi", "GetGameSettingApi"),
            ("/NetDelivery/GetGameSettingApi?k=v", "GetGameSettingApi"),
            ("NetDelivery\\GetGameSettingApi", "GetGameSettingApi"),
            ("GetGameSettingApi", "GetGameSettingApi"),
        ];

        // When/Then: 都取到末段逻辑名。
        for (path, expected) in cases {
            assert_eq!(api_name_from_path(path).as_deref(), Some(expected));
        }
    }

    #[test]
    fn empty_path_has_no_api_name() {
        // Given: 空路径。
        // Then: 不产生逻辑名，调用方跳过处理。
        assert!(api_name_from_path("").is_none());
        assert!(api_name_from_path("/").is_none());
    }

    #[test]
    fn dispatch_ignores_non_json_bodies() {
        // Given: 一段非 JSON 响应。
        // When/Then: 派发不 panic 且不触发任何处理（无订阅者时也安全）。
        dispatch("/x/GetGameSettingApi", b"not json at all");
        dispatch("/x/GetGameSettingApi", &[0xff, 0xfe, 0x00]);
    }
}
