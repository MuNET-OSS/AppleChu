use std::ffi::c_void;
use std::net::TcpStream;
use std::sync::Mutex;

use once_cell::sync::OnceCell;
use windows_sys::Win32::Networking::WinSock::{SOCKADDR, SOCKADDR_IN, WSABUF};

mod music;

use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

const WS2_32: &str = "ws2_32.dll";

const TYPE_HOLDPUNCH: u8 = 0;
const TYPE_REFLECTOR: u8 = 1;
const TYPE_TUNNEL: u8 = 2;
const TYPE_MUSIC: u8 = 4;

const ISFINISH_FALSE: &[u8] = b"{\"isFinish\":false}\0";

type WsaSendToFn = unsafe extern "system" fn(
    usize,
    *const WSABUF,
    u32,
    *mut u32,
    u32,
    *const SOCKADDR,
    i32,
    *mut c_void,
    *mut c_void,
) -> i32;

type WsaRecvFromFn = unsafe extern "system" fn(
    usize,
    *const WSABUF,
    u32,
    *mut u32,
    *mut u32,
    *mut SOCKADDR,
    *mut i32,
    *mut c_void,
    *mut c_void,
) -> i32;

struct State {
    reflector: Mutex<Option<TcpStream>>,
    reflector_addr: Mutex<Option<[u8; 4]>>,
    reflector_port: Mutex<u16>,
    music_sent: Mutex<bool>,
}

static STATE: OnceCell<State> = OnceCell::new();
static ORIG_SENDTO: OnceCell<WsaSendToFn> = OnceCell::new();
static ORIG_RECVFROM: OnceCell<WsaRecvFromFn> = OnceCell::new();

crate::config_section! {
    pub(crate) struct NationalMatchConfig => NATIONAL_MATCH_CONFIG_SECTION {
        section: "NationalMatch",
        order: 185,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "全国对战 TCP 中继",
        fields: {}
    }
}

#[applechu_macros::config_section(stage = Late, order = 10)]
pub fn init(api: &Api, _config: &NationalMatchConfig) {
    let _ = STATE.set(State {
        reflector: Mutex::new(None),
        reflector_addr: Mutex::new(None),
        reflector_port: Mutex::new(0),
        music_sent: Mutex::new(false),
    });

    unsafe {
        if let Some(orig) = hook_iat(
            api.game_base(),
            WS2_32,
            "WSASendTo",
            hooked_sendto as *const (),
        ) {
            let _ = ORIG_SENDTO.set(std::mem::transmute::<*const (), WsaSendToFn>(orig));
        } else {
            api.log_warn("national match: WSASendTo import not found");
        }

        if let Some(orig) = hook_iat(
            api.game_base(),
            WS2_32,
            "WSARecvFrom",
            hooked_recvfrom as *const (),
        ) {
            let _ = ORIG_RECVFROM.set(std::mem::transmute::<*const (), WsaRecvFromFn>(orig));
        } else {
            api.log_warn("national match: WSARecvFrom import not found");
        }
    }

    api.log_info("national match: enabled (UDP<->TCP relay)");

    music::init(api);
}

unsafe fn collect_payload(buffers: &[WSABUF]) -> Option<Vec<u8>> {
    let capacity = buffers
        .iter()
        .filter(|buffer| !buffer.buf.is_null())
        .try_fold(0usize, |length, buffer| {
            length.checked_add(buffer.len as usize)
        })?;
    if capacity > isize::MAX as usize {
        return None;
    }
    let mut data = Vec::new();
    data.try_reserve_exact(capacity).ok()?;
    for buf in buffers {
        if !buf.buf.is_null() && buf.len > 0 {
            // SAFETY: WSASendTo 保证每个非空 WSABUF 在调用期间至少包含 len 字节
            let bytes = unsafe { std::slice::from_raw_parts(buf.buf, buf.len as usize) };
            data.extend_from_slice(bytes);
        }
    }
    Some(data)
}

fn descriptor_count(buffers: *const WSABUF, count: u32) -> Option<usize> {
    if count == 0 {
        return Some(0);
    }
    if buffers.is_null() {
        return None;
    }
    let count = count as usize;
    if count > isize::MAX as usize / std::mem::size_of::<WSABUF>() {
        return None;
    }
    Some(count)
}

unsafe fn sockaddr_ipv4(addr: *const SOCKADDR, addr_len: i32) -> Option<([u8; 4], u16)> {
    if addr.is_null() || addr_len < std::mem::size_of::<SOCKADDR_IN>() as i32 {
        return None;
    }
    // SAFETY: 上方已验证非空指针和调用方声明的缓冲区长度
    unsafe {
        let sin = &*(addr as *const SOCKADDR_IN);
        let ip = sin.sin_addr.S_un.S_addr.to_ne_bytes();
        let port = u16::from_be(sin.sin_port);
        Some((ip, port))
    }
}

fn build_frame(packet_type: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut frame = Vec::with_capacity(3 + len);
    frame.push(packet_type);
    frame.push((len & 0xFF) as u8);
    frame.push(((len >> 8) & 0xFF) as u8);
    frame.extend_from_slice(payload);
    frame
}

fn send_to_reflector(frame: &[u8]) -> bool {
    use std::io::Write;
    let Some(state) = STATE.get() else {
        return false;
    };

    let mut guard = state
        .reflector
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        let addr = state
            .reflector_addr
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let port = *state
            .reflector_port
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(ip) = *addr {
            let target = std::net::SocketAddr::from((ip, port));
            if let Ok(stream) = TcpStream::connect(target) {
                let _ = stream.set_nodelay(true);
                *guard = Some(stream);
            }
        }
    }

    if let Some(stream) = guard.as_mut() {
        if stream.write_all(frame).is_ok() {
            return true;
        }
    }
    *guard = None;
    false
}

unsafe extern "system" fn hooked_sendto(
    socket: usize,
    buffers: *const WSABUF,
    buffer_count: u32,
    bytes_sent: *mut u32,
    flags: u32,
    to: *const SOCKADDR,
    to_len: i32,
    overlapped: *mut c_void,
    completion: *mut c_void,
) -> i32 {
    let Some(state) = STATE.get() else {
        return passthrough_sendto(
            socket,
            buffers,
            buffer_count,
            bytes_sent,
            flags,
            to,
            to_len,
            overlapped,
            completion,
        );
    };

    let Some(descriptor_count) = descriptor_count(buffers, buffer_count) else {
        return passthrough_sendto(
            socket,
            buffers,
            buffer_count,
            bytes_sent,
            flags,
            to,
            to_len,
            overlapped,
            completion,
        );
    };
    let descriptors = if descriptor_count == 0 {
        &[]
    } else {
        // SAFETY: WSASendTo 保证 buffers 在调用期间包含 buffer_count 个有效描述符
        unsafe { std::slice::from_raw_parts(buffers, descriptor_count) }
    };
    // SAFETY: WSASendTo 保证每个描述符的非空缓冲区在调用期间至少包含 len 字节
    let Some(payload) = (unsafe { collect_payload(descriptors) }) else {
        return passthrough_sendto(
            socket,
            buffers,
            buffer_count,
            bytes_sent,
            flags,
            to,
            to_len,
            overlapped,
            completion,
        );
    };
    // SAFETY: WSASendTo 保证 to 在非空时指向长度为 to_len 的可读地址结构
    let dest = unsafe { sockaddr_ipv4(to, to_len) };

    let is_holdpunch = payload.len() >= 4 && &payload[0..4] == b"{\"ro";

    if is_holdpunch {
        if let Some((ip, port)) = dest {
            *state
                .reflector_addr
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(ip);
            *state
                .reflector_port
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = port;
        }
        let frame = build_frame(TYPE_HOLDPUNCH, &payload);
        send_to_reflector(&frame);

        let already = {
            let mut sent = state
                .music_sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let was = *sent;
            *sent = true;
            was
        };
        if !already {
            let payload = music::music_payload();
            let music = build_frame(TYPE_MUSIC, &payload);
            send_to_reflector(&music);
        }

        report_sent(bytes_sent, payload.len());
        return 0;
    }

    let reflector_ip = *state
        .reflector_addr
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let (Some((ip, port)), Some(refl)) = (dest, reflector_ip) {
        let frame = if ip == refl {
            build_frame(TYPE_REFLECTOR, &payload)
        } else {
            let mut tunnel = Vec::with_capacity(6 + payload.len());
            tunnel.extend_from_slice(&ip);
            tunnel.extend_from_slice(&port.to_be_bytes());
            tunnel.extend_from_slice(&payload);
            build_frame(TYPE_TUNNEL, &tunnel)
        };
        send_to_reflector(&frame);
        report_sent(bytes_sent, payload.len());
        return 0;
    }

    passthrough_sendto(
        socket,
        buffers,
        buffer_count,
        bytes_sent,
        flags,
        to,
        to_len,
        overlapped,
        completion,
    )
}

unsafe fn report_sent(bytes_sent: *mut u32, len: usize) {
    if !bytes_sent.is_null() {
        *bytes_sent = len as u32;
    }
}

unsafe fn passthrough_sendto(
    socket: usize,
    buffers: *const WSABUF,
    buffer_count: u32,
    bytes_sent: *mut u32,
    flags: u32,
    to: *const SOCKADDR,
    to_len: i32,
    overlapped: *mut c_void,
    completion: *mut c_void,
) -> i32 {
    if let Some(orig) = ORIG_SENDTO.get() {
        return orig(
            socket,
            buffers,
            buffer_count,
            bytes_sent,
            flags,
            to,
            to_len,
            overlapped,
            completion,
        );
    }
    0
}

unsafe extern "system" fn hooked_recvfrom(
    socket: usize,
    buffers: *const WSABUF,
    buffer_count: u32,
    bytes_recvd: *mut u32,
    flags: *mut u32,
    from: *mut SOCKADDR,
    from_len: *mut i32,
    overlapped: *mut c_void,
    completion: *mut c_void,
) -> i32 {
    let Some(state) = STATE.get() else {
        return passthrough_recvfrom(
            socket,
            buffers,
            buffer_count,
            bytes_recvd,
            flags,
            from,
            from_len,
            overlapped,
            completion,
        );
    };

    let frame = match recv_frame(state) {
        Some(f) => f,
        None => {
            return passthrough_recvfrom(
                socket,
                buffers,
                buffer_count,
                bytes_recvd,
                flags,
                from,
                from_len,
                overlapped,
                completion,
            );
        }
    };

    let (packet_type, payload) = frame;
    let reflector_ip = *state
        .reflector_addr
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let reflector_ip = reflector_ip.unwrap_or([0, 0, 0, 0]);
    let reflector_port = *state
        .reflector_port
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    match packet_type {
        TYPE_TUNNEL if payload.len() >= 6 => {
            let ip = [payload[0], payload[1], payload[2], payload[3]];
            let port = u16::from_be_bytes([payload[4], payload[5]]);
            write_from(from, from_len, ip, port);
            deliver(buffers, buffer_count, bytes_recvd, &payload[6..]);
        }
        TYPE_MUSIC => {
            music::apply_intersection(&payload);
            write_from(from, from_len, reflector_ip, reflector_port);
            deliver(buffers, buffer_count, bytes_recvd, ISFINISH_FALSE);
        }
        _ => {
            write_from(from, from_len, reflector_ip, reflector_port);
            deliver(buffers, buffer_count, bytes_recvd, &payload);
        }
    }

    0
}

fn recv_frame(state: &State) -> Option<(u8, Vec<u8>)> {
    use std::io::Read;
    let mut guard = state
        .reflector
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stream = guard.as_mut()?;

    let mut header = [0u8; 3];
    if stream.read_exact(&mut header).is_err() {
        *guard = None;
        return None;
    }

    let packet_type = header[0];
    let len = (header[1] as usize) | ((header[2] as usize) << 8);
    let mut payload = vec![0u8; len];
    if len > 0 && stream.read_exact(&mut payload).is_err() {
        *guard = None;
        return None;
    }
    Some((packet_type, payload))
}

unsafe fn write_from(from: *mut SOCKADDR, from_len: *mut i32, ip: [u8; 4], port: u16) {
    let required = std::mem::size_of::<SOCKADDR_IN>() as i32;
    if from.is_null() || from_len.is_null() || *from_len < required {
        return;
    }
    let sin = &mut *(from as *mut SOCKADDR_IN);
    sin.sin_family = windows_sys::Win32::Networking::WinSock::AF_INET;
    sin.sin_port = port.to_be();
    sin.sin_addr.S_un.S_addr = u32::from_ne_bytes(ip);
    *from_len = required;
}

unsafe fn deliver(buffers: *const WSABUF, buffer_count: u32, bytes_recvd: *mut u32, data: &[u8]) {
    let mut written = 0usize;
    let Some(descriptor_count) = descriptor_count(buffers, buffer_count) else {
        if !bytes_recvd.is_null() {
            *bytes_recvd = 0;
        }
        return;
    };
    let descriptors = if descriptor_count == 0 {
        &[]
    } else {
        // SAFETY: WSARecvFrom 保证 buffers 在调用期间包含 buffer_count 个有效描述符
        unsafe { std::slice::from_raw_parts(buffers, descriptor_count) }
    };
    for buf in descriptors {
        if written >= data.len() {
            break;
        }
        if buf.buf.is_null() || buf.len == 0 {
            continue;
        }
        let take = std::cmp::min(buf.len as usize, data.len() - written);
        // SAFETY: WSARecvFrom 保证每个非空 WSABUF 在调用期间至少可写入 len 字节
        std::ptr::copy_nonoverlapping(data.as_ptr().add(written), buf.buf, take);
        written += take;
    }
    if !bytes_recvd.is_null() {
        *bytes_recvd = written as u32;
    }
}

unsafe fn passthrough_recvfrom(
    socket: usize,
    buffers: *const WSABUF,
    buffer_count: u32,
    bytes_recvd: *mut u32,
    flags: *mut u32,
    from: *mut SOCKADDR,
    from_len: *mut i32,
    overlapped: *mut c_void,
    completion: *mut c_void,
) -> i32 {
    if let Some(orig) = ORIG_RECVFROM.get() {
        return orig(
            socket,
            buffers,
            buffer_count,
            bytes_recvd,
            flags,
            from,
            from_len,
            overlapped,
            completion,
        );
    }
    -1
}

#[cfg(test)]
mod tests {
    use super::{build_frame, collect_payload, descriptor_count};
    use windows_sys::Win32::Networking::WinSock::WSABUF;

    #[test]
    fn reflector_frame_keeps_type_length_and_payload() {
        assert_eq!(build_frame(2, &[0xAA, 0xBB]), [2, 2, 0, 0xAA, 0xBB]);
    }

    #[test]
    fn payload_collection_preserves_descriptor_order() {
        let mut first = [1u8, 2];
        let mut second = [3u8, 4, 5];
        let buffers = [
            WSABUF {
                len: first.len() as u32,
                buf: first.as_mut_ptr(),
            },
            WSABUF {
                len: 0,
                buf: std::ptr::null_mut(),
            },
            WSABUF {
                len: second.len() as u32,
                buf: second.as_mut_ptr(),
            },
        ];

        // SAFETY: 测试描述符引用上方仍在作用域内的数组
        assert_eq!(
            unsafe { collect_payload(&buffers) },
            Some(vec![1, 2, 3, 4, 5])
        );
    }

    #[test]
    fn nonempty_descriptor_list_rejects_null_pointer() {
        assert!(descriptor_count(std::ptr::null(), 1).is_none());
    }
}
