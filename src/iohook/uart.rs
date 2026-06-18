use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::ptr;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use windows_sys::Win32::Foundation::HANDLE;

use crate::config::Config;
use crate::util::api::Api;

const FAKE_HANDLE_BASE: usize = 0x434F_0000;

pub const IOCTL_SERIAL_SET_BAUD_RATE: u32 = 0x001B_0004;
pub const IOCTL_SERIAL_GET_BAUD_RATE: u32 = 0x001B_0008;
pub const IOCTL_SERIAL_SET_QUEUE_SIZE: u32 = 0x001B_000C;
pub const IOCTL_SERIAL_SET_LINE_CONTROL: u32 = 0x001B_001C;
pub const IOCTL_SERIAL_GET_LINE_CONTROL: u32 = 0x001B_0020;
pub const IOCTL_SERIAL_SET_TIMEOUTS: u32 = 0x001B_0024;
pub const IOCTL_SERIAL_GET_TIMEOUTS: u32 = 0x001B_0028;
pub const IOCTL_SERIAL_SET_DTR: u32 = 0x001B_002C;
pub const IOCTL_SERIAL_CLR_DTR: u32 = 0x001B_0030;
pub const IOCTL_SERIAL_RESET_DEVICE: u32 = 0x001B_0034;
pub const IOCTL_SERIAL_SET_RTS: u32 = 0x001B_0038;
pub const IOCTL_SERIAL_CLR_RTS: u32 = 0x001B_003C;
pub const IOCTL_SERIAL_SET_CHARS: u32 = 0x001B_005C;
pub const IOCTL_SERIAL_GET_CHARS: u32 = 0x001B_0060;
pub const IOCTL_SERIAL_SET_HANDFLOW: u32 = 0x001B_0064;
pub const IOCTL_SERIAL_GET_HANDFLOW: u32 = 0x001B_0068;
pub const IOCTL_SERIAL_PURGE: u32 = 0x001B_004C;

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

#[derive(Clone)]
pub struct UartSnapshot {
    pub baud_rate: u32,
    pub line_control: SerialLineControl,
    pub timeouts: SerialTimeouts,
    pub handflow: SerialHandflow,
}

struct UartPort {
    rx: VecDeque<u8>,
    tx: Vec<u8>,
    baud_rate: u32,
    line_control: SerialLineControl,
    timeouts: SerialTimeouts,
    handflow: SerialHandflow,
    dtr: bool,
    rts: bool,
}

impl Default for UartPort {
    fn default() -> Self {
        Self {
            rx: VecDeque::new(),
            tx: Vec::new(),
            baud_rate: 115_200,
            line_control: SerialLineControl {
                stop_bits: 0,
                parity: 0,
                word_length: 8,
            },
            timeouts: SerialTimeouts::default(),
            handflow: SerialHandflow::default(),
            dtr: false,
            rts: false,
        }
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
    if handle >= FAKE_HANDLE_BASE && handle < FAKE_HANDLE_BASE + 0x10000 {
        return true;
    }
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
                irp.fd = fake_handle(port_no) as HANDLE;
                1
            } else {
                -1
            }
        }
        crate::iohook::IrpOp::Close => is_uart_handle(irp.fd as usize) as i32 * 2 - 1,
        crate::iohook::IrpOp::Read => {
            if !is_uart_handle(irp.fd as usize) {
                return -1;
            }
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = 0;
            }
            if irp.read_buf.is_null() || irp.nbytes == 0 {
                return 1;
            }
            let read = with_port(irp.fd as usize, |port| {
                let out = std::slice::from_raw_parts_mut(irp.read_buf, irp.nbytes as usize);
                let mut count = 0usize;
                while count < out.len() {
                    let Some(value) = port.rx.pop_front() else {
                        break;
                    };
                    out[count] = value;
                    count += 1;
                }
                count as u32
            })
            .unwrap_or(0);
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = read;
            }
            1
        }
        crate::iohook::IrpOp::Write => {
            if !is_uart_handle(irp.fd as usize) {
                return -1;
            }
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = irp.nbytes;
            }
            if !irp.write_buf.is_null() && irp.nbytes != 0 {
                let data = std::slice::from_raw_parts(irp.write_buf, irp.nbytes as usize);
                let _ = with_port(irp.fd as usize, |port| port.tx.extend_from_slice(data));
            }
            1
        }
        crate::iohook::IrpOp::Ioctl => {
            if is_uart_handle(irp.fd as usize) {
                device_io_control(
                    irp.fd as usize,
                    irp.ioctl,
                    irp.ioctl_in,
                    irp.ioctl_in_nbytes,
                    irp.ioctl_out,
                    irp.ioctl_out_nbytes,
                    irp.out_nbytes,
                )
            } else {
                -1
            }
        }
        crate::iohook::IrpOp::Fsync | crate::iohook::IrpOp::Seek => {
            if is_uart_handle(irp.fd as usize) {
                1
            } else {
                -1
            }
        }
    }
}

pub fn snapshot(handle: usize) -> Option<UartSnapshot> {
    with_port(handle, |port| UartSnapshot {
        baud_rate: port.baud_rate,
        line_control: port.line_control,
        timeouts: port.timeouts,
        handflow: port.handflow,
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
        port.rx.clear();
        port.tx.clear();
    })
    .is_some()
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
        return 0;
    }
    if !bytes_returned.is_null() {
        *bytes_returned = 0;
    }

    match code {
        IOCTL_SERIAL_SET_BAUD_RATE => read_input::<SerialBaudRate>(in_buffer, in_size)
            .is_some_and(|value| set_baud_rate(handle, value.baud_rate))
            as i32,
        IOCTL_SERIAL_GET_BAUD_RATE => write_output(
            out_buffer,
            out_size,
            bytes_returned,
            &SerialBaudRate {
                baud_rate: snapshot(handle).map_or(115_200, |state| state.baud_rate),
            },
        ) as i32,
        IOCTL_SERIAL_SET_LINE_CONTROL => read_input::<SerialLineControl>(in_buffer, in_size)
            .is_some_and(|value| set_line_control(handle, value))
            as i32,
        IOCTL_SERIAL_GET_LINE_CONTROL => snapshot(handle).is_some_and(|state| {
            write_output(out_buffer, out_size, bytes_returned, &state.line_control)
        }) as i32,
        IOCTL_SERIAL_SET_TIMEOUTS => read_input::<SerialTimeouts>(in_buffer, in_size)
            .is_some_and(|value| set_timeouts(handle, value))
            as i32,
        IOCTL_SERIAL_GET_TIMEOUTS => snapshot(handle).is_some_and(|state| {
            write_output(out_buffer, out_size, bytes_returned, &state.timeouts)
        }) as i32,
        IOCTL_SERIAL_SET_HANDFLOW => read_input::<SerialHandflow>(in_buffer, in_size)
            .is_some_and(|value| set_handflow(handle, value))
            as i32,
        IOCTL_SERIAL_GET_HANDFLOW => snapshot(handle).is_some_and(|state| {
            write_output(out_buffer, out_size, bytes_returned, &state.handflow)
        }) as i32,
        IOCTL_SERIAL_SET_DTR => set_escape(handle, 6) as i32,
        IOCTL_SERIAL_CLR_DTR => set_escape(handle, 7) as i32,
        IOCTL_SERIAL_SET_RTS => set_escape(handle, 5) as i32,
        IOCTL_SERIAL_CLR_RTS => set_escape(handle, 4) as i32,
        IOCTL_SERIAL_PURGE
        | IOCTL_SERIAL_RESET_DEVICE
        | IOCTL_SERIAL_SET_QUEUE_SIZE
        | IOCTL_SERIAL_SET_CHARS => 1,
        IOCTL_SERIAL_GET_CHARS => {
            let chars = [0u8; 6];
            write_output(out_buffer, out_size, bytes_returned, &chars) as i32
        }
        _ => 1,
    }
}

fn with_port<T>(handle: usize, action: impl FnOnce(&mut UartPort) -> T) -> Option<T> {
    let port_no = port_from_handle(handle)?;
    let Ok(mut ports) = PORTS.lock() else {
        return None;
    };
    ports.get_mut(&port_no).map(action)
}

pub fn fake_handle_pub(port_no: u32) -> usize {
    fake_handle(port_no)
}

fn fake_handle(port_no: u32) -> usize {
    FAKE_HANDLE_BASE | port_no as usize
}

fn port_from_handle(handle: usize) -> Option<u32> {
    if handle >= FAKE_HANDLE_BASE && handle < FAKE_HANDLE_BASE + 0x10000 {
        return Some((handle & 0xFFFF) as u32);
    }
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
