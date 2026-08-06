use std::ffi::{c_char, c_void, CString};
use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::OnceCell;
use windows_sys::Win32::Foundation::{HANDLE, HMODULE};

use applechu::amdaemon::DnsConfig;
use applechu::iohook::hook_table::{
    hook_table_apply, hook_table_apply_ordinals, null_module, HookSymbol, OrdinalHookSymbol,
};
use applechu::iohook::proc_addr;
use applechu::util::api::Api;

type DnsQueryAFn = unsafe extern "system" fn(
    *const c_char,
    u16,
    u32,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> i32;
type DnsQueryWFn =
    unsafe extern "system" fn(*const u16, u16, u32, *mut c_void, *mut c_void, *mut c_void) -> i32;
type DnsQueryExFn =
    unsafe extern "system" fn(*mut DnsQueryRequest, *mut c_void, *mut c_void) -> i32;
type GetAddrInfoAFn =
    unsafe extern "system" fn(*const c_char, *const c_char, *const c_void, *mut *mut c_void) -> i32;
type GetAddrInfoWFn =
    unsafe extern "system" fn(*const u16, *const u16, *const c_void, *mut *mut c_void) -> i32;
type GetAddrInfoExAFn = unsafe extern "system" fn(
    *const c_char,
    *const c_char,
    u32,
    *const c_void,
    *const c_void,
    *mut *mut c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
    *mut HANDLE,
) -> i32;
type GetAddrInfoExWFn = unsafe extern "system" fn(
    *const u16,
    *const u16,
    u32,
    *const c_void,
    *const c_void,
    *mut *mut c_void,
    *const c_void,
    *mut c_void,
    *mut c_void,
    *mut HANDLE,
) -> i32;
type WinHttpConnectFn = unsafe extern "system" fn(*mut c_void, *const u16, u16, u32) -> *mut c_void;
type WinHttpCrackUrlFn = unsafe extern "system" fn(*const u16, u32, u32, *mut c_void) -> i32;
type ConnectFn = unsafe extern "system" fn(usize, *const SockAddrIn, i32) -> i32;

#[repr(C)]
struct DnsQueryRequest {
    version: u32,
    query_name: *const u16,
    query_type: u16,
    query_options: u64,
    dns_server_list: *mut c_void,
    interface_index: u32,
    completion_callback: *mut c_void,
    query_context: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrIn {
    family: u16,
    port: u16,
    address: u32,
    zero: [u8; 8],
}

static CONFIG: OnceCell<DnsConfig> = OnceCell::new();
static ORIG_DNS_QUERY_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_DNS_QUERY_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_DNS_QUERY_EX: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_ADDR_INFO_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_ADDR_INFO_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_ADDR_INFO_EX_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_ADDR_INFO_EX_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_WINHTTP_CONNECT: AtomicUsize = AtomicUsize::new(0);
static ORIG_WINHTTP_CRACK_URL: AtomicUsize = AtomicUsize::new(0);
static ORIG_CONNECT: AtomicUsize = AtomicUsize::new(0);
static mut ORIG_DNS_QUERY_A_PTR: *const () = std::ptr::null();
static mut ORIG_DNS_QUERY_W_PTR: *const () = std::ptr::null();
static mut ORIG_DNS_QUERY_EX_PTR: *const () = std::ptr::null();
static mut ORIG_GET_ADDR_INFO_A_PTR: *const () = std::ptr::null();
static mut ORIG_GET_ADDR_INFO_W_PTR: *const () = std::ptr::null();
static mut ORIG_GET_ADDR_INFO_EX_A_PTR: *const () = std::ptr::null();
static mut ORIG_GET_ADDR_INFO_EX_W_PTR: *const () = std::ptr::null();
static mut ORIG_WINHTTP_CONNECT_PTR: *const () = std::ptr::null();
static mut ORIG_WINHTTP_CRACK_URL_PTR: *const () = std::ptr::null();
static mut ORIG_CONNECT_PTR: *const () = std::ptr::null();

#[applechu_macros::config_section(stage = Platform, order = 30)]
pub fn init(api: &Api, config: &DnsConfig) {
    let resolved = resolve_config(config);
    let _ = CONFIG.set(resolved.clone());
    unsafe {
        let dns = dns_symbols();
        let ws2 = ws2_symbols();
        let winhttp = winhttp_symbols();
        proc_addr::push("dnsapi.dll", &dns, sync_originals);
        proc_addr::push("ws2_32.dll", &ws2, sync_originals);
        proc_addr::push("winhttp.dll", &winhttp, sync_originals);
        let patched = hook_table_apply(null_module(), "dnsapi.dll", &dns)
            + hook_table_apply(null_module(), "ws2_32.dll", &ws2)
            + hook_table_apply(null_module(), "winhttp.dll", &winhttp)
            + hook_table_apply_ordinals(null_module(), "ws2_32.dll", &getaddrinfo_ordinal_symbol());
        sync_originals();
        let port_patched = if resolved.startup_port != 0
            || resolved.billing_port != 0
            || resolved.aimedb_port != 0
        {
            let port_symbols = [HookSymbol {
                name: "connect",
                patch: hooked_connect as *const (),
                original: std::ptr::addr_of_mut!(ORIG_CONNECT_PTR),
            }];
            proc_addr::push("ws2_32.dll", &port_symbols, sync_originals);
            let patched = hook_table_apply(null_module(), "ws2_32.dll", &port_symbols);
            sync_originals();
            patched
        } else {
            0
        };
        api.log_info(&format!(
            "DNS redirection enabled with {} patched entries",
            patched + port_patched
        ));
    }
}

fn resolve_config(config: &DnsConfig) -> DnsConfig {
    let mut resolved = config.clone();
    for target in [
        &mut resolved.router,
        &mut resolved.startup,
        &mut resolved.billing,
        &mut resolved.aimedb,
    ] {
        if target.is_empty() {
            target.clone_from(&resolved.default);
        }
    }
    resolved
}

/// 为平台 Hook 阶段之后加载的 DLL 应用 DNS 重定向
pub unsafe fn apply_hooks(module: HMODULE) -> usize {
    let dns = dns_symbols();
    let ws2 = ws2_symbols();
    let winhttp = winhttp_symbols();
    let patched = hook_table_apply(module, "dnsapi.dll", &dns)
        + hook_table_apply(module, "ws2_32.dll", &ws2)
        + hook_table_apply(module, "winhttp.dll", &winhttp)
        + hook_table_apply_ordinals(module, "ws2_32.dll", &getaddrinfo_ordinal_symbol());
    sync_originals();
    patched
}

unsafe fn dns_symbols() -> [HookSymbol; 3] {
    [
        HookSymbol {
            name: "DnsQuery_A",
            patch: hooked_dns_query_a as *const (),
            original: std::ptr::addr_of_mut!(ORIG_DNS_QUERY_A_PTR),
        },
        HookSymbol {
            name: "DnsQuery_W",
            patch: hooked_dns_query_w as *const (),
            original: std::ptr::addr_of_mut!(ORIG_DNS_QUERY_W_PTR),
        },
        HookSymbol {
            name: "DnsQueryEx",
            patch: hooked_dns_query_ex as *const (),
            original: std::ptr::addr_of_mut!(ORIG_DNS_QUERY_EX_PTR),
        },
    ]
}

unsafe fn ws2_symbols() -> [HookSymbol; 4] {
    [
        HookSymbol {
            name: "getaddrinfo",
            patch: hooked_get_addr_info_a as *const (),
            original: std::ptr::addr_of_mut!(ORIG_GET_ADDR_INFO_A_PTR),
        },
        HookSymbol {
            name: "GetAddrInfoW",
            patch: hooked_get_addr_info_w as *const (),
            original: std::ptr::addr_of_mut!(ORIG_GET_ADDR_INFO_W_PTR),
        },
        HookSymbol {
            name: "GetAddrInfoExA",
            patch: hooked_get_addr_info_ex_a as *const (),
            original: std::ptr::addr_of_mut!(ORIG_GET_ADDR_INFO_EX_A_PTR),
        },
        HookSymbol {
            name: "GetAddrInfoExW",
            patch: hooked_get_addr_info_ex_w as *const (),
            original: std::ptr::addr_of_mut!(ORIG_GET_ADDR_INFO_EX_W_PTR),
        },
    ]
}

unsafe fn winhttp_symbols() -> [HookSymbol; 2] {
    [
        HookSymbol {
            name: "WinHttpConnect",
            patch: hooked_winhttp_connect as *const (),
            original: std::ptr::addr_of_mut!(ORIG_WINHTTP_CONNECT_PTR),
        },
        HookSymbol {
            name: "WinHttpCrackUrl",
            patch: hooked_winhttp_crack_url as *const (),
            original: std::ptr::addr_of_mut!(ORIG_WINHTTP_CRACK_URL_PTR),
        },
    ]
}

unsafe fn getaddrinfo_ordinal_symbol() -> [OrdinalHookSymbol; 1] {
    [OrdinalHookSymbol {
        ordinal: 176,
        patch: hooked_get_addr_info_a as *const (),
        original: std::ptr::addr_of_mut!(ORIG_GET_ADDR_INFO_A_PTR),
    }]
}

unsafe extern "system" fn hooked_dns_query_a(
    name: *const c_char,
    query_type: u16,
    options: u32,
    extra: *mut c_void,
    results: *mut c_void,
    reserved: *mut c_void,
) -> i32 {
    let original = original::<DnsQueryAFn>(&ORIG_DNS_QUERY_A);
    let Some(query) = applechu::platform::winapi::cstr_to_string(name) else {
        return 87;
    };
    match replacement(&query) {
        Some(Some(host)) => {
            log_mapping("DnsQuery_A", &query, Some(host));
            let Ok(host) = CString::new(host) else {
                return 87;
            };
            original(host.as_ptr(), query_type, options, extra, results, reserved)
        }
        Some(None) => {
            log_mapping("DnsQuery_A", &query, None);
            9003
        }
        None => original(name, query_type, options, extra, results, reserved),
    }
}

unsafe extern "system" fn hooked_dns_query_w(
    name_ptr: *const u16,
    query_type: u16,
    options: u32,
    extra: *mut c_void,
    results: *mut c_void,
    reserved: *mut c_void,
) -> i32 {
    let original = original::<DnsQueryWFn>(&ORIG_DNS_QUERY_W);
    let Some(name) = applechu::platform::winapi::wide_to_string(name_ptr) else {
        return 87;
    };
    match replacement(&name) {
        Some(Some(host)) => {
            log_mapping("DnsQuery_W", &name, Some(host));
            let host = applechu::platform::winapi::string_to_wide(host);
            original(host.as_ptr(), query_type, options, extra, results, reserved)
        }
        Some(None) => {
            log_mapping("DnsQuery_W", &name, None);
            9003
        }
        None => original(name_ptr, query_type, options, extra, results, reserved),
    }
}

unsafe extern "system" fn hooked_dns_query_ex(
    request: *mut DnsQueryRequest,
    results: *mut c_void,
    cancel: *mut c_void,
) -> i32 {
    if request.is_null() {
        return 87;
    }
    let original = original::<DnsQueryExFn>(&ORIG_DNS_QUERY_EX);
    let Some(name) = applechu::platform::winapi::wide_to_string((*request).query_name) else {
        return 87;
    };
    let Some(replacement) = replacement(&name) else {
        return original(request, results, cancel);
    };
    let Some(host) = replacement else {
        log_mapping("DnsQueryEx", &name, None);
        return 9003;
    };

    log_mapping("DnsQueryEx", &name, Some(host));
    let host = applechu::platform::winapi::string_to_wide(host);
    let previous = (*request).query_name;
    (*request).query_name = host.as_ptr();
    let result = original(request, results, cancel);
    (*request).query_name = previous;
    result
}

unsafe extern "system" fn hooked_get_addr_info_a(
    node: *const c_char,
    service: *const c_char,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    let original = original::<GetAddrInfoAFn>(&ORIG_GET_ADDR_INFO_A);
    let Some(name) = applechu::platform::winapi::cstr_to_string(node) else {
        return 10022;
    };
    match replacement(&name) {
        Some(Some(host)) => {
            log_mapping("getaddrinfo", &name, Some(host));
            let Ok(host) = CString::new(host) else {
                return 10022;
            };
            original(host.as_ptr(), service, hints, result)
        }
        Some(None) => {
            log_mapping("getaddrinfo", &name, None);
            11001
        }
        None => original(node, service, hints, result),
    }
}

unsafe extern "system" fn hooked_get_addr_info_w(
    node: *const u16,
    service: *const u16,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    let original = original::<GetAddrInfoWFn>(&ORIG_GET_ADDR_INFO_W);
    let Some(name) = applechu::platform::winapi::wide_to_string(node) else {
        return 10022;
    };
    match replacement(&name) {
        Some(Some(host)) => {
            log_mapping("GetAddrInfoW", &name, Some(host));
            let host = applechu::platform::winapi::string_to_wide(host);
            original(host.as_ptr(), service, hints, result)
        }
        Some(None) => {
            log_mapping("GetAddrInfoW", &name, None);
            11001
        }
        None => original(node, service, hints, result),
    }
}

unsafe extern "system" fn hooked_get_addr_info_ex_a(
    node: *const c_char,
    service: *const c_char,
    namespace: u32,
    namespace_id: *const c_void,
    hints: *const c_void,
    result: *mut *mut c_void,
    timeout: *const c_void,
    overlapped: *mut c_void,
    completion: *mut c_void,
    handle: *mut HANDLE,
) -> i32 {
    let original = original::<GetAddrInfoExAFn>(&ORIG_GET_ADDR_INFO_EX_A);
    let Some(name) = applechu::platform::winapi::cstr_to_string(node) else {
        return 10022;
    };
    match replacement(&name) {
        Some(Some(host)) => {
            log_mapping("GetAddrInfoExA", &name, Some(host));
            let Ok(host) = CString::new(host) else {
                return 10022;
            };
            original(
                host.as_ptr(),
                service,
                namespace,
                namespace_id,
                hints,
                result,
                timeout,
                overlapped,
                completion,
                handle,
            )
        }
        Some(None) => {
            log_mapping("GetAddrInfoExA", &name, None);
            11001
        }
        None => original(
            node,
            service,
            namespace,
            namespace_id,
            hints,
            result,
            timeout,
            overlapped,
            completion,
            handle,
        ),
    }
}

unsafe extern "system" fn hooked_get_addr_info_ex_w(
    node: *const u16,
    service: *const u16,
    namespace: u32,
    namespace_id: *const c_void,
    hints: *const c_void,
    result: *mut *mut c_void,
    timeout: *const c_void,
    overlapped: *mut c_void,
    completion: *mut c_void,
    handle: *mut HANDLE,
) -> i32 {
    let original = original::<GetAddrInfoExWFn>(&ORIG_GET_ADDR_INFO_EX_W);
    let Some(name) = applechu::platform::winapi::wide_to_string(node) else {
        return 10022;
    };
    match replacement(&name) {
        Some(Some(host)) => {
            log_mapping("GetAddrInfoExW", &name, Some(host));
            let host = applechu::platform::winapi::string_to_wide(host);
            original(
                host.as_ptr(),
                service,
                namespace,
                namespace_id,
                hints,
                result,
                timeout,
                overlapped,
                completion,
                handle,
            )
        }
        Some(None) => {
            log_mapping("GetAddrInfoExW", &name, None);
            11001
        }
        None => original(
            node,
            service,
            namespace,
            namespace_id,
            hints,
            result,
            timeout,
            overlapped,
            completion,
            handle,
        ),
    }
}

unsafe extern "system" fn hooked_winhttp_connect(
    session: *mut c_void,
    server: *const u16,
    port: u16,
    reserved: u32,
) -> *mut c_void {
    let original = original::<WinHttpConnectFn>(&ORIG_WINHTTP_CONNECT);
    let Some(name) = applechu::platform::winapi::wide_to_string(server) else {
        return std::ptr::null_mut();
    };
    match replacement(&name) {
        Some(Some(host)) => {
            log_mapping("WinHttpConnect", &name, Some(host));
            let host = applechu::platform::winapi::string_to_wide(host);
            original(session, host.as_ptr(), port, reserved)
        }
        Some(None) => {
            log_mapping("WinHttpConnect", &name, None);
            std::ptr::null_mut()
        }
        None => original(session, server, port, reserved),
    }
}

unsafe extern "system" fn hooked_winhttp_crack_url(
    url: *const u16,
    url_length: u32,
    flags: u32,
    components: *mut c_void,
) -> i32 {
    let original = original::<WinHttpCrackUrlFn>(&ORIG_WINHTTP_CRACK_URL);
    let Some(name) = wide_with_length(url, url_length) else {
        return original(url, url_length, flags, components);
    };
    match replacement(&name) {
        Some(Some(target)) => {
            log_mapping("WinHttpCrackUrl", &name, Some(target));
            let target = applechu::platform::winapi::string_to_wide(target);
            original(
                target.as_ptr(),
                target.len().saturating_sub(1) as u32,
                flags,
                components,
            )
        }
        Some(None) => {
            log_mapping("WinHttpCrackUrl", &name, None);
            0
        }
        None => original(url, url_length, flags, components),
    }
}

unsafe fn wide_with_length(value: *const u16, length: u32) -> Option<String> {
    if value.is_null() {
        return None;
    }
    if length == 0 {
        return applechu::platform::winapi::wide_to_string(value);
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(
        value,
        length as usize,
    )))
}

fn log_mapping(api: &str, source: &str, target: Option<&str>) {
    let target = target.unwrap_or("blocked");
    if let Some(logger) = applechu::util::api::API.get() {
        logger.log_info(&format!("DNS: {source} -> {target} ({api})"));
    }
}

fn replacement(name: &str) -> Option<Option<&str>> {
    let config = CONFIG.get()?;
    let name = name.trim_end_matches('.');
    let target = if matches_domain(name, "tenporouter.loc") || matches_domain(name, "bbrouter.loc")
    {
        Some(&config.router)
    } else if matches_domain(name, "ib.naominet.jp") || matches_domain(name, "vo.anbzvarg.wc") {
        Some(&config.billing)
    } else if matches_domain(name, "aime.naominet.jp") || matches_domain(name, "nvzr.anbzvarg.wc") {
        Some(&config.aimedb)
    } else if matches_domain(name, "*.amlog.sys-all.net")
        || matches_domain(name, "*.d-amlog.sys-all.net")
        || matches_domain(name, "mobirouter.loc")
        || matches_domain(name, "dslrouter.loc")
    {
        return Some(None);
    } else if [
        "naominet.jp",
        "anbzvarg.wc",
        "op.auth.sys-all.net",
        "at.auth.sys-all.net",
        "at.sys-all.net",
        "sega-initiald.net",
        "api-aime.am-all.net",
        "tasms-api-basis.thincacloud.com",
        "shop.tfps.thincacloud.com",
        "at.sys-all.cn",
        "at.sys-allnet.cn",
    ]
    .iter()
    .any(|pattern| matches_domain(name, pattern))
    {
        Some(&config.startup)
    } else if matches_domain(name, "https://rev-ent.ac.capcom.jp:443") {
        Some(&config.title)
    } else if ["ai.sys-all.cn", "ai.sys-allnet.cn"]
        .iter()
        .any(|pattern| matches_domain(name, pattern))
    {
        Some(&config.aimedb)
    } else if ["bl.sys-all.cn", "bl.sys-allnet.cn"]
        .iter()
        .any(|pattern| matches_domain(name, pattern))
    {
        Some(&config.billing)
    } else {
        None
    };
    target.map(|host| (!host.is_empty()).then_some(host.as_str()))
}

fn matches_domain(name: &str, pattern: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let pattern = pattern.to_ascii_lowercase();
    match pattern.strip_prefix("*.") {
        Some(suffix) => name == suffix || name.ends_with(&format!(".{suffix}")),
        None => name == pattern,
    }
}

fn sync_originals() {
    unsafe {
        ORIG_DNS_QUERY_A.store(ORIG_DNS_QUERY_A_PTR as usize, Ordering::Release);
        ORIG_DNS_QUERY_W.store(ORIG_DNS_QUERY_W_PTR as usize, Ordering::Release);
        ORIG_DNS_QUERY_EX.store(ORIG_DNS_QUERY_EX_PTR as usize, Ordering::Release);
        ORIG_GET_ADDR_INFO_A.store(ORIG_GET_ADDR_INFO_A_PTR as usize, Ordering::Release);
        ORIG_GET_ADDR_INFO_W.store(ORIG_GET_ADDR_INFO_W_PTR as usize, Ordering::Release);
        ORIG_GET_ADDR_INFO_EX_A.store(ORIG_GET_ADDR_INFO_EX_A_PTR as usize, Ordering::Release);
        ORIG_GET_ADDR_INFO_EX_W.store(ORIG_GET_ADDR_INFO_EX_W_PTR as usize, Ordering::Release);
        ORIG_WINHTTP_CONNECT.store(ORIG_WINHTTP_CONNECT_PTR as usize, Ordering::Release);
        ORIG_WINHTTP_CRACK_URL.store(ORIG_WINHTTP_CRACK_URL_PTR as usize, Ordering::Release);
        ORIG_CONNECT.store(ORIG_CONNECT_PTR as usize, Ordering::Release);
    }
}

unsafe extern "system" fn hooked_connect(
    socket: usize,
    address: *const SockAddrIn,
    address_len: i32,
) -> i32 {
    let original = original::<ConnectFn>(&ORIG_CONNECT);
    if address.is_null() || address_len < std::mem::size_of::<SockAddrIn>() as i32 {
        return original(socket, address, address_len);
    }
    let config = CONFIG.get();
    let Some(config) = config else {
        return original(socket, address, address_len);
    };
    let source = *address;
    if source.family != 2 {
        return original(socket, address, address_len);
    }
    let port = u16::from_be(source.port);
    let replacement = match port {
        80 if config.startup_port != 0 => Some(config.startup_port),
        8443 if config.billing_port != 0 => Some(config.billing_port),
        22345 if config.aimedb_port != 0 => Some(config.aimedb_port),
        _ => None,
    };
    let Some(replacement) = replacement else {
        return original(socket, address, address_len);
    };
    let mut target = source;
    target.port = replacement.to_be();
    original(socket, &target, std::mem::size_of::<SockAddrIn>() as i32)
}

unsafe fn original<T>(slot: &AtomicUsize) -> T {
    std::mem::transmute_copy(&slot.load(Ordering::Acquire))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_host_fills_unspecified_service_hosts() {
        let config = DnsConfig {
            default: "play.example.com".to_owned(),
            aimedb: "aime.example.com".to_owned(),
            ..DnsConfig::default()
        };

        let resolved = resolve_config(&config);

        assert_eq!(resolved.router, "play.example.com");
        assert_eq!(resolved.startup, "play.example.com");
        assert_eq!(resolved.billing, "play.example.com");
        assert_eq!(resolved.aimedb, "aime.example.com");
    }

    #[test]
    fn empty_default_keeps_unspecified_service_hosts_empty() {
        let resolved = resolve_config(&DnsConfig::default());

        assert!(resolved.router.is_empty());
        assert!(resolved.startup.is_empty());
        assert!(resolved.billing.is_empty());
        assert!(resolved.aimedb.is_empty());
    }

    #[test]
    fn matches_exact_names_and_subdomains() {
        assert!(matches_domain("op.auth.sys-all.net", "op.auth.sys-all.net"));
        assert!(matches_domain(
            "log.amlog.sys-all.net",
            "*.amlog.sys-all.net"
        ));
        assert!(!matches_domain(
            "amlog.sys-all.net.example",
            "*.amlog.sys-all.net"
        ));
    }
}
