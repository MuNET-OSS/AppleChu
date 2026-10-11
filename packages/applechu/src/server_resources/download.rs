//! 资源包下载
//!
//! 使用进程内的 WinHTTP 直接下载，不经过游戏的连接池，也不硬编码服务器地址
//! （地址来自服务器下发的 `entry.url`）。对应 AquaMai `GetFileImpl` 的下载段。

use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_DEFAULTS,
};

/// 单个响应体的上限，避免异常响应耗尽内存
const MAX_DOWNLOAD: usize = 256 * 1024 * 1024;
/// 网络超时（毫秒）：连接 / 发送 / 接收
const TIMEOUT_MS: i32 = 30_000;
const WINHTTP_OPTION_CONNECT_TIMEOUT: u32 = 3;
const WINHTTP_OPTION_SEND_TIMEOUT: u32 = 4;
const WINHTTP_OPTION_RECEIVE_TIMEOUT: u32 = 5;

/// 下载 `url` 指向的资源包
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let parsed = ParsedUrl::parse(url)?;
    let agent_name = crate::platform::winapi::string_to_wide("AppleChu");
    let server = crate::platform::winapi::string_to_wide(&parsed.host);
    let path = crate::platform::winapi::string_to_wide(&parsed.path_and_query);

    unsafe {
        let session = WinHttpOpen(
            agent_name.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if session.is_null() {
            return Err("WinHttpOpen failed".to_owned());
        }

        let result = (|| -> Result<Vec<u8>, String> {
            apply_timeout(session)?;

            let connect = WinHttpConnect(session, server.as_ptr(), parsed.port, 0);
            if connect.is_null() {
                return Err(format!("WinHttpConnect failed for {}", parsed.host));
            }

            let request_result = (|| -> Result<Vec<u8>, String> {
                let flags = if parsed.secure {
                    WINHTTP_FLAG_SECURE | WINHTTP_FLAG_SECURE_DEFAULTS
                } else {
                    0
                };
                let request = WinHttpOpenRequest(
                    connect,
                    ptr::null(),
                    path.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    flags,
                );
                if request.is_null() {
                    return Err("WinHttpOpenRequest failed".to_owned());
                }

                let body_result = (|| -> Result<Vec<u8>, String> {
                    if WinHttpSendRequest(request, ptr::null(), 0, ptr::null(), 0, 0, 0) == 0 {
                        return Err("WinHttpSendRequest failed".to_owned());
                    }
                    if WinHttpReceiveResponse(request, ptr::null_mut()) == 0 {
                        return Err("WinHttpReceiveResponse failed".to_owned());
                    }

                    let status = query_status(request);
                    if !(200..300).contains(&status) {
                        return Err(format!("download returned HTTP {status}"));
                    }

                    let mut body = Vec::new();
                    loop {
                        let mut available = 0u32;
                        if WinHttpQueryDataAvailable(request, &mut available) == 0 {
                            return Err("WinHttpQueryDataAvailable failed".to_owned());
                        }
                        if available == 0 {
                            break;
                        }
                        let mut chunk = vec![0u8; available as usize];
                        let mut read = 0u32;
                        if WinHttpReadData(
                            request,
                            chunk.as_mut_ptr().cast::<c_void>(),
                            available,
                            &mut read,
                        ) == 0
                        {
                            return Err("WinHttpReadData failed".to_owned());
                        }
                        if read == 0 {
                            break;
                        }
                        if body.len() + read as usize > MAX_DOWNLOAD {
                            return Err("download exceeded size limit".to_owned());
                        }
                        chunk.truncate(read as usize);
                        body.extend_from_slice(&chunk);
                    }
                    Ok(body)
                })();

                WinHttpCloseHandle(request);
                body_result
            })();

            WinHttpCloseHandle(connect);
            request_result
        })();

        WinHttpCloseHandle(session);
        result
    }
}

/// 设置连接/发送/接收超时；失败不致命，仅在无超时保护下继续
unsafe fn apply_timeout(session: *mut c_void) -> Result<(), String> {
    use windows_sys::Win32::Networking::WinHttp::WinHttpSetOption;
    let value = TIMEOUT_MS;
    for option in [
        WINHTTP_OPTION_CONNECT_TIMEOUT,
        WINHTTP_OPTION_SEND_TIMEOUT,
        WINHTTP_OPTION_RECEIVE_TIMEOUT,
    ] {
        let ok = WinHttpSetOption(
            session,
            option,
            (&value as *const i32).cast::<c_void>(),
            std::mem::size_of::<i32>() as u32,
        );
        if ok == 0 {
            return Err(format!("WinHttpSetOption({option}) failed"));
        }
    }
    Ok(())
}

/// 读取 HTTP 状态码；失败按 0 处理，交由范围判断拒绝
unsafe fn query_status(request: *mut c_void) -> u32 {
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpQueryHeaders, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    };
    let mut status = 0u32;
    let mut length = std::mem::size_of::<u32>() as u32;
    let ok = WinHttpQueryHeaders(
        request,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
        ptr::null(),
        (&mut status as *mut u32).cast::<c_void>(),
        &mut length,
        ptr::null_mut(),
    );
    if ok == 0 {
        0
    } else {
        status
    }
}

/// 仅支持 http/https 的简化 URL 分解
struct ParsedUrl {
    secure: bool,
    host: String,
    port: u16,
    path_and_query: String,
}

impl ParsedUrl {
    fn parse(url: &str) -> Result<Self, String> {
        let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(format!("unsupported url scheme: {url}"));
        };

        let (authority, path_and_query) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, "/"),
        };
        if authority.is_empty() {
            return Err(format!("url has no host: {url}"));
        }

        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| format!("invalid port in url: {url}"))?;
                (host.to_owned(), port)
            }
            None => (authority.to_owned(), if secure { 443 } else { 80 }),
        };

        Ok(Self {
            secure,
            host,
            port,
            path_and_query: path_and_query.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url_with_default_port() {
        // Given: 无显式端口的 https 地址。
        let parsed = ParsedUrl::parse("https://play.mumet.net/pack/option.bin").unwrap();

        // Then: 使用 443 且保留完整路径。
        assert!(parsed.secure);
        assert_eq!(parsed.host, "play.mumet.net");
        assert_eq!(parsed.port, 443);
        assert_eq!(parsed.path_and_query, "/pack/option.bin");
    }

    #[test]
    fn parses_http_url_with_query_and_explicit_port() {
        // Given: 带查询串与显式端口的 http 地址。
        let parsed = ParsedUrl::parse("http://127.0.0.1:8080/a/b?c=d").unwrap();

        // Then: 端口与查询串被正确分离。
        assert!(!parsed.secure);
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, 8080);
        assert_eq!(parsed.path_and_query, "/a/b?c=d");
    }

    #[test]
    fn missing_path_defaults_to_root() {
        // Given: 只有主机名的地址。
        let parsed = ParsedUrl::parse("https://example.com").unwrap();

        // Then: 路径补为 `/`。
        assert_eq!(parsed.path_and_query, "/");
    }

    #[test]
    fn rejects_unsupported_scheme() {
        // Given: 非 http(s) 地址。
        // Then: 解析失败，避免把任意协议交给 WinHTTP。
        assert!(ParsedUrl::parse("ftp://example.com/a").is_err());
        assert!(ParsedUrl::parse("file:///c:/x").is_err());
    }
}
