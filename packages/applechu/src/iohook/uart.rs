use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::config::Config;
use crate::iohook;
use crate::util::api::Api;

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

pub const IOCTL_SERIAL_SET_BAUD_RATE: u32 = 0x001B_0004;
pub const IOCTL_SERIAL_SET_QUEUE_SIZE: u32 = 0x001B_0008;
pub const IOCTL_SERIAL_SET_LINE_CONTROL: u32 = 0x001B_000C;
pub const IOCTL_SERIAL_SET_BREAK_ON: u32 = 0x001B_0010;
pub const IOCTL_SERIAL_SET_BREAK_OFF: u32 = 0x001B_0014;
pub const IOCTL_SERIAL_SET_TIMEOUTS: u32 = 0x001B_001C;
pub const IOCTL_SERIAL_GET_TIMEOUTS: u32 = 0x001B_0020;
pub const IOCTL_SERIAL_SET_DTR: u32 = 0x001B_0024;
pub const IOCTL_SERIAL_CLR_DTR: u32 = 0x001B_0028;
pub const IOCTL_SERIAL_RESET_DEVICE: u32 = 0x001B_002C;
pub const IOCTL_SERIAL_SET_RTS: u32 = 0x001B_0030;
pub const IOCTL_SERIAL_CLR_RTS: u32 = 0x001B_0034;
pub const IOCTL_SERIAL_SET_XOFF: u32 = 0x001B_0038;
pub const IOCTL_SERIAL_SET_XON: u32 = 0x001B_003C;
pub const IOCTL_SERIAL_GET_WAIT_MASK: u32 = 0x001B_0040;
pub const IOCTL_SERIAL_SET_WAIT_MASK: u32 = 0x001B_0044;
pub const IOCTL_SERIAL_PURGE: u32 = 0x001B_004C;
pub const IOCTL_SERIAL_GET_BAUD_RATE: u32 = 0x001B_0050;
pub const IOCTL_SERIAL_GET_LINE_CONTROL: u32 = 0x001B_0054;
pub const IOCTL_SERIAL_GET_CHARS: u32 = 0x001B_0058;
pub const IOCTL_SERIAL_SET_CHARS: u32 = 0x001B_005C;
pub const IOCTL_SERIAL_GET_HANDFLOW: u32 = 0x001B_0060;
pub const IOCTL_SERIAL_SET_HANDFLOW: u32 = 0x001B_0064;
pub const IOCTL_SERIAL_GET_MODEMSTATUS: u32 = 0x001B_0068;
pub const IOCTL_SERIAL_GET_COMMSTATUS: u32 = 0x001B_006C;
pub const IOCTL_SERIAL_GET_MODEM_CONTROL: u32 = 0x001B_0094;
pub const IOCTL_SERIAL_SET_MODEM_CONTROL: u32 = 0x001B_0098;

static PORTS: Lazy<Mutex<HashMap<u32, UartPort>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static HANDLE_PORTS: Lazy<Mutex<HashMap<usize, u32>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SerialBaudRate {
    pub baud_rate: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SerialLineControl {
    pub stop_bits: u8,
    pub parity: u8,
    pub word_length: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SerialTimeouts {
    pub read_interval_timeout: u32,
    pub read_total_timeout_multiplier: u32,
    pub read_total_timeout_constant: u32,
    pub write_total_timeout_multiplier: u32,
    pub write_total_timeout_constant: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SerialHandflow {
    pub control_handshake: u32,
    pub flow_replace: u32,
    pub xon_limit: i32,
    pub xoff_limit: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SerialChars {
    pub eof_char: u8,
    pub error_char: u8,
    pub break_char: u8,
    pub event_char: u8,
    pub xon_char: u8,
    pub xoff_char: u8,
}

impl Default for SerialChars {
    fn default() -> Self {
        Self {
            eof_char: 0,
            error_char: 0,
            break_char: 0,
            event_char: 0,
            xon_char: 0x11,
            xoff_char: 0x13,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SerialStatus {
    pub errors: u32,
    pub hold_reasons: u32,
    pub amount_in_in_queue: u32,
    pub amount_in_out_queue: u32,
    pub eof_received: u8,
    pub wait_for_immediate: u8,
}

#[derive(Clone)]
pub struct UartSnapshot {
    pub baud_rate: u32,
    pub line_control: SerialLineControl,
    pub timeouts: SerialTimeouts,
    pub handflow: SerialHandflow,
    pub chars: SerialChars,
    pub wait_mask: u32,
    pub modem_control: u32,
    pub modem_status: u32,
    pub input_queue: u32,
    pub output_queue: u32,
}

struct UartPort {
    readable: Vec<u8>,
    written: Vec<u8>,
    baud_rate: u32,
    line_control: SerialLineControl,
    timeouts: SerialTimeouts,
    handflow: SerialHandflow,
    chars: SerialChars,
    wait_mask: u32,
    modem_control: u32,
    cts: bool,
    dsr: bool,
    ring: bool,
    rlsd: bool,
    dtr: bool,
    rts: bool,
}

impl Default for UartPort {
    fn default() -> Self {
        Self {
            readable: Vec::new(),
            written: Vec::new(),
            baud_rate: 115_200,
            line_control: SerialLineControl {
                stop_bits: 0,
                parity: 0,
                word_length: 8,
            },
            timeouts: SerialTimeouts::default(),
            handflow: SerialHandflow::default(),
            chars: SerialChars::default(),
            wait_mask: 0,
            modem_control: 0,
            cts: false,
            dsr: false,
            ring: false,
            rlsd: false,
            dtr: false,
            rts: false,
        }
    }
}

impl UartPort {
    fn push_readable_bounded(&mut self, bytes: &[u8], capacity: usize) -> bool {
        let Some(required) = self.readable.len().checked_add(bytes.len()) else {
            return false;
        };
        if required > capacity {
            return false;
        }
        self.readable.extend_from_slice(bytes);
        true
    }
}

pub fn init_all(_api: &Api, _config: &Config) {}

pub fn uart_init(port_no: u32) {
    let Ok(mut ports) = PORTS.lock() else {
        return;
    };
    ports.entry(port_no).or_insert_with(UartPort::default);
}

pub fn is_uart_handle(handle: usize) -> bool {
    HANDLE_PORTS
        .lock()
        .is_ok_and(|handles| handles.contains_key(&handle))
}

pub fn bind_handle(handle: usize, port_no: u32) {
    uart_init(port_no);
    if let Ok(mut handles) = HANDLE_PORTS.lock() {
        handles.insert(handle, port_no);
    }
}

pub fn unbind_handle(handle: usize) {
    if let Ok(mut handles) = HANDLE_PORTS.lock() {
        handles.remove(&handle);
    }
}

pub unsafe fn uart_handle_irp(irp: &mut crate::iohook::Irp) -> i32 {
    match irp.op {
        crate::iohook::IrpOp::Open => {
            let port_no =
                parse_com_a(irp.open_filename_a).or_else(|| parse_com_w(irp.open_filename_w));
            if let Some(port_no) = port_no.filter(|port_no| port_registered(*port_no)) {
                let Some(fd) = crate::iohook::open_nul_fd() else {
                    return iohook::E_FAIL;
                };
                let handle = crate::util::win32::handle_value(fd);
                bind_handle(handle, port_no);
                irp.fd = fd;
                iohook::S_OK
            } else {
                iohook::invoke_next(irp)
            }
        }
        crate::iohook::IrpOp::Close => {
            let handle = crate::util::win32::handle_value(irp.fd);
            if is_uart_handle(handle) {
                unbind_handle(handle);
                // 清理本地串口状态后继续钩子链
                crate::iohook::invoke_next(irp)
            } else {
                crate::iohook::invoke_next(irp)
            }
        }
        crate::iohook::IrpOp::Read => {
            if !is_uart_handle(crate::util::win32::handle_value(irp.fd)) {
                return iohook::invoke_next(irp);
            }
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = 0;
            }
            if irp.read_buf.is_null() || irp.nbytes == 0 {
                return iohook::S_OK;
            }
            let read = with_port(crate::util::win32::handle_value(irp.fd), |port| {
                let out = std::slice::from_raw_parts_mut(irp.read_buf, irp.nbytes as usize);
                let count = out.len().min(port.readable.len());
                out[..count].copy_from_slice(&port.readable[..count]);
                port.readable.drain(..count);
                count as u32
            })
            .unwrap_or(0);
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = read;
            }
            if read == 0 && irp.nbytes != 0 {
                iohook::E_PENDING
            } else {
                iohook::S_OK
            }
        }
        crate::iohook::IrpOp::Write => {
            if !is_uart_handle(crate::util::win32::handle_value(irp.fd)) {
                return iohook::invoke_next(irp);
            }
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = irp.nbytes;
            }
            if !irp.write_buf.is_null() && irp.nbytes != 0 {
                let data = std::slice::from_raw_parts(irp.write_buf, irp.nbytes as usize);
                let _ = with_port(crate::util::win32::handle_value(irp.fd), |port| {
                    port.written.extend_from_slice(data)
                });
            }
            iohook::S_OK
        }
        crate::iohook::IrpOp::Ioctl => {
            if is_uart_handle(crate::util::win32::handle_value(irp.fd)) {
                device_io_control(
                    crate::util::win32::handle_value(irp.fd),
                    irp.ioctl,
                    irp.ioctl_in,
                    irp.ioctl_in_nbytes,
                    irp.ioctl_out,
                    irp.ioctl_out_nbytes,
                    irp.out_nbytes,
                )
            } else {
                iohook::invoke_next(irp)
            }
        }
        crate::iohook::IrpOp::Fsync => {
            if is_uart_handle(crate::util::win32::handle_value(irp.fd)) {
                iohook::S_OK
            } else {
                iohook::invoke_next(irp)
            }
        }
        crate::iohook::IrpOp::Seek => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
    }
}

pub fn snapshot(handle: usize) -> Option<UartSnapshot> {
    with_port(handle, |port| UartSnapshot {
        baud_rate: port.baud_rate,
        line_control: port.line_control,
        timeouts: port.timeouts,
        handflow: port.handflow,
        chars: port.chars,
        wait_mask: port.wait_mask,
        modem_control: port.modem_control,
        modem_status: (u32::from(port.cts) << 4)
            | (u32::from(port.dsr) << 5)
            | (u32::from(port.ring) << 6)
            | (u32::from(port.rlsd) << 7),
        input_queue: port.readable.len() as u32,
        output_queue: port.written.len() as u32,
    })
}

pub fn set_baud_rate(handle: usize, baud_rate: u32) -> bool {
    with_port(handle, |port| port.baud_rate = baud_rate).is_some()
}

pub fn set_line_control(handle: usize, line_control: SerialLineControl) -> bool {
    with_port(handle, |port| port.line_control = line_control).is_some()
}

pub fn set_timeouts(handle: usize, timeouts: SerialTimeouts) -> bool {
    with_port(handle, |port| port.timeouts = timeouts).is_some()
}

pub fn set_handflow(handle: usize, handflow: SerialHandflow) -> bool {
    with_port(handle, |port| port.handflow = handflow).is_some()
}

pub fn set_chars(handle: usize, chars: SerialChars) -> bool {
    with_port(handle, |port| port.chars = chars).is_some()
}

pub fn set_wait_mask(handle: usize, mask: u32) -> bool {
    with_port(handle, |port| port.wait_mask = mask).is_some()
}

pub fn set_modem_control(handle: usize, control: u32) -> bool {
    with_port(handle, |port| port.modem_control = control).is_some()
}

pub fn set_escape(handle: usize, function: u32) -> bool {
    with_port(handle, |port| match function {
        5 => port.rts = true,
        4 => port.rts = false,
        6 => port.dtr = true,
        7 => port.dtr = false,
        _ => {}
    })
    .is_some()
}

pub fn purge(handle: usize) -> bool {
    with_port(handle, |port| {
        port.readable.clear();
        port.written.clear();
    })
    .is_some()
}

/// 取出设备尚未解析的请求字节。解析完成后必须调用 `restore_written` 放回残帧
pub fn take_written(handle: usize) -> Option<Vec<u8>> {
    with_port(handle, |port| std::mem::take(&mut port.written))
}

/// 将未解析的残帧放回请求队列头部，并保留处理期间新到达的字节
pub fn restore_written(handle: usize, mut pending: Vec<u8>) -> bool {
    with_port(handle, |port| {
        pending.append(&mut port.written);
        port.written = pending;
    })
    .is_some()
}

/// 将设备回复写入公共 UART 可读队列
pub fn push_readable(handle: usize, bytes: &[u8]) -> bool {
    with_port(handle, |port| port.readable.extend_from_slice(bytes)).is_some()
}

/// 异步设备回调没有文件句柄时，按端口写入公共 UART 可读队列
pub fn push_readable_port(port_no: u32, bytes: &[u8]) -> bool {
    let Ok(mut ports) = PORTS.lock() else {
        return false;
    };
    let Some(port) = ports.get_mut(&port_no) else {
        return false;
    };
    port.readable.extend_from_slice(bytes);
    true
}

/// 按完整消息写入有界 UART 可读队列，空间不足时不写入
pub fn push_readable_port_bounded(port_no: u32, bytes: &[u8], capacity: usize) -> bool {
    let Ok(mut ports) = PORTS.lock() else {
        return false;
    };
    let Some(port) = ports.get_mut(&port_no) else {
        return false;
    };
    port.push_readable_bounded(bytes, capacity)
}

pub unsafe fn device_io_control(
    handle: usize,
    code: u32,
    in_buffer: *mut c_void,
    in_size: u32,
    out_buffer: *mut c_void,
    out_size: u32,
    bytes_returned: *mut u32,
) -> i32 {
    if !is_uart_handle(handle) {
        return iohook::hresult_from_win32(ERROR_INVALID_FUNCTION);
    }
    if !bytes_returned.is_null() {
        *bytes_returned = 0;
    }

    match code {
        IOCTL_SERIAL_SET_BAUD_RATE => {
            update_from_input(in_buffer, in_size, |value: SerialBaudRate| {
                set_baud_rate(handle, value.baud_rate)
            })
        }
        IOCTL_SERIAL_GET_BAUD_RATE => write_output_hresult(
            out_buffer,
            out_size,
            bytes_returned,
            &SerialBaudRate {
                baud_rate: snapshot(handle).map_or(115_200, |state| state.baud_rate),
            },
        ),
        IOCTL_SERIAL_SET_LINE_CONTROL => {
            update_from_input(in_buffer, in_size, |value| set_line_control(handle, value))
        }
        IOCTL_SERIAL_GET_LINE_CONTROL => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.line_control)
        }),
        IOCTL_SERIAL_SET_TIMEOUTS => {
            update_from_input(in_buffer, in_size, |value| set_timeouts(handle, value))
        }
        IOCTL_SERIAL_GET_TIMEOUTS => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.timeouts)
        }),
        IOCTL_SERIAL_SET_HANDFLOW => {
            update_from_input(in_buffer, in_size, |value| set_handflow(handle, value))
        }
        IOCTL_SERIAL_GET_HANDFLOW => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.handflow)
        }),
        IOCTL_SERIAL_GET_CHARS => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.chars)
        }),
        IOCTL_SERIAL_SET_CHARS => {
            update_from_input(in_buffer, in_size, |value| set_chars(handle, value))
        }
        IOCTL_SERIAL_GET_WAIT_MASK => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.wait_mask)
        }),
        IOCTL_SERIAL_SET_WAIT_MASK => {
            update_from_input(in_buffer, in_size, |value| set_wait_mask(handle, value))
        }
        IOCTL_SERIAL_GET_MODEM_CONTROL => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.modem_control)
        }),
        IOCTL_SERIAL_SET_MODEM_CONTROL => {
            update_from_input(in_buffer, in_size, |value| set_modem_control(handle, value))
        }
        IOCTL_SERIAL_GET_MODEMSTATUS => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            write_output_hresult(out_buffer, out_size, bytes_returned, &state.modem_status)
        }),
        IOCTL_SERIAL_GET_COMMSTATUS => snapshot(handle).map_or(iohook::E_FAIL, |state| {
            let status = SerialStatus {
                amount_in_in_queue: state.input_queue,
                amount_in_out_queue: state.output_queue,
                ..SerialStatus::default()
            };
            write_output_hresult(out_buffer, out_size, bytes_returned, &status)
        }),
        IOCTL_SERIAL_SET_DTR => bool_hresult(set_escape(handle, 6)),
        IOCTL_SERIAL_CLR_DTR => bool_hresult(set_escape(handle, 7)),
        IOCTL_SERIAL_SET_RTS => bool_hresult(set_escape(handle, 5)),
        IOCTL_SERIAL_CLR_RTS => bool_hresult(set_escape(handle, 4)),
        IOCTL_SERIAL_PURGE
        | IOCTL_SERIAL_SET_QUEUE_SIZE
        | IOCTL_SERIAL_SET_BREAK_ON
        | IOCTL_SERIAL_SET_BREAK_OFF
        | IOCTL_SERIAL_SET_XOFF
        | IOCTL_SERIAL_SET_XON => iohook::S_OK,
        _ => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
    }
}

unsafe fn update_from_input<T: Copy>(
    buffer: *mut c_void,
    size: u32,
    update: impl FnOnce(T) -> bool,
) -> i32 {
    let Some(value) = read_input::<T>(buffer, size) else {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    };
    bool_hresult(update(value))
}

fn bool_hresult(ok: bool) -> i32 {
    if ok {
        iohook::S_OK
    } else {
        iohook::E_FAIL
    }
}

fn with_port<T>(handle: usize, action: impl FnOnce(&mut UartPort) -> T) -> Option<T> {
    let port_no = port_from_handle(handle)?;
    let Ok(mut ports) = PORTS.lock() else {
        return None;
    };
    ports.get_mut(&port_no).map(action)
}

fn port_from_handle(handle: usize) -> Option<u32> {
    HANDLE_PORTS.lock().ok()?.get(&handle).copied()
}

fn port_registered(port_no: u32) -> bool {
    PORTS.lock().is_ok_and(|ports| ports.contains_key(&port_no))
}

pub unsafe fn parse_com_a(ptr: *const u8) -> Option<u32> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 && len < 64 {
        len += 1;
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len))
        .ok()
        .and_then(parse_com_name)
}

pub unsafe fn parse_com_w(ptr: *const u16) -> Option<u32> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 && len < 64 {
        len += 1;
    }
    String::from_utf16(std::slice::from_raw_parts(ptr, len))
        .ok()
        .and_then(|name| parse_com_name(&name))
}

fn parse_com_name(name: &str) -> Option<u32> {
    let trimmed = name
        .strip_prefix("\\\\.\\")
        .or_else(|| name.strip_prefix("\\\\?\\"))
        .or_else(|| name.strip_prefix("\\??\\"))
        .unwrap_or(name);
    let suffix = trimmed
        .strip_prefix("COM")
        .or_else(|| trimmed.strip_prefix("com"))?
        .strip_suffix(':')
        .unwrap_or_else(|| {
            trimmed
                .strip_prefix("COM")
                .or_else(|| trimmed.strip_prefix("com"))
                .unwrap_or("")
        });
    let port_no = suffix.parse::<u32>().ok()?;
    (1..=256).contains(&port_no).then_some(port_no)
}

unsafe fn read_input<T: Copy>(buffer: *mut c_void, size: u32) -> Option<T> {
    if buffer.is_null() || size < std::mem::size_of::<T>() as u32 {
        return None;
    }
    Some(ptr::read_unaligned(buffer.cast::<T>()))
}

unsafe fn write_output<T: Copy>(
    buffer: *mut c_void,
    size: u32,
    bytes_returned: *mut u32,
    value: &T,
) -> bool {
    let needed = std::mem::size_of::<T>() as u32;
    if buffer.is_null() || size < needed {
        return false;
    }
    ptr::write_unaligned(buffer.cast::<T>(), *value);
    if !bytes_returned.is_null() {
        *bytes_returned = needed;
    }
    true
}

unsafe fn write_output_hresult<T: Copy>(
    buffer: *mut c_void,
    size: u32,
    bytes_returned: *mut u32,
    value: &T,
) -> i32 {
    if write_output(buffer, size, bytes_returned, value) {
        iohook::S_OK
    } else {
        iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER)
    }
}

const _: () = assert!(std::mem::size_of::<SerialBaudRate>() == 4);
const _: () = assert!(std::mem::size_of::<SerialLineControl>() == 3);
const _: () = assert!(std::mem::size_of::<SerialChars>() == 6);
const _: () = assert!(std::mem::size_of::<SerialHandflow>() == 16);
const _: () = assert!(std::mem::size_of::<SerialTimeouts>() == 20);
const _: () = assert!(std::mem::size_of::<SerialStatus>() == 20);

#[cfg(test)]
mod tests {
    use super::UartPort;

    #[test]
    fn bounded_readable_queue_drops_whole_messages() {
        let mut port = UartPort::default();
        let frame = [0xA5; 36];

        for _ in 0..14 {
            assert!(port.push_readable_bounded(&frame, 520));
        }
        assert_eq!(port.readable.len(), 504);

        assert!(!port.push_readable_bounded(&frame, 520));
        assert_eq!(port.readable.len(), 504);
        assert!(port
            .readable
            .chunks_exact(frame.len())
            .all(|item| item == frame));
    }

    #[test]
    fn bounded_readable_queue_resumes_after_data_is_read() {
        let mut port = UartPort::default();
        let first = [0x11; 260];
        let second = [0x22; 260];

        assert!(port.push_readable_bounded(&first, 520));
        assert!(port.push_readable_bounded(&second, 520));
        assert!(!port.push_readable_bounded(&second, 520));

        port.readable.drain(..first.len());
        assert!(port.push_readable_bounded(&first, 520));
        assert_eq!(&port.readable[..second.len()], &second);
        assert_eq!(&port.readable[second.len()..], &first);
    }
}
