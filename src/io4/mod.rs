pub mod chusan_io4;

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::thread;

use once_cell::sync::Lazy;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::System::Threading::SetEvent;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
};

use crate::config::Config;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct Io4Config => IO4_CONFIG_SECTION {
        section: "Io4",
        order: 310,
        default_enabled: true,
        always_enabled: false,
        hidden: false,
        comment: "IO4 USB HID 模拟",
        fields: {
            pub foreground: bool = true,
            comment: "仅在游戏窗口位于前台时读取输入";
        }
    }
}

pub const BUTTON_TEST: u16 = 1 << 9;
pub const BUTTON_SERVICE: u16 = 1 << 6;
pub const REPORT_LEN: usize = 0x40;
const OUT_PAYLOAD_LEN: usize = 62;
const IO4_PATH: &str = "$io4\\vid_0ca3";
const IOCTL_HID_GET_MANUFACTURER_STRING: u32 = 0xB019_0012;
const IOCTL_HID_GET_PRODUCT_STRING: u32 = 0xB019_0016;
const IOCTL_HID_GET_INPUT_REPORT: u32 = 0xB019_0008;
const IOCTL_HID_SET_OUTPUT_REPORT: u32 = 0xB019_0009;
const ERROR_IO_PENDING: u32 = 997;
const STATUS_PENDING: usize = 0x0000_0103;
const STATUS_SUCCESS: usize = 0;

static IO4_FD: Mutex<HANDLE> = Mutex::new(0);
static IO4_DEVICE: Lazy<Mutex<Option<Io4Device<chusan_io4::ChusanIo4Ops>>>> =
    Lazy::new(|| Mutex::new(None));
static IO4_ASYNC: OnceLock<Sender<Io4AsyncRead>> = OnceLock::new();

#[derive(Clone, Copy, Default)]
pub struct Io4State {
    pub adcs: [u16; 8],
    pub spinners: [u16; 4],
    pub chutes: [u16; 2],
    pub buttons: [u16; 2],
}

pub trait Io4Ops: Send + Sync + 'static {
    fn poll(&self) -> Io4State;

    fn write_gpio(&self, _payload: &[u8]) -> Result<(), i32> {
        Ok(())
    }

    fn write_pwm(&self, _payload: &[u8]) -> Result<(), i32> {
        Ok(())
    }

    fn write_unique(&self, _payload: &[u8]) -> Result<(), i32> {
        Ok(())
    }
}

pub struct Io4Device<O> {
    ops: O,
    foreground_only: bool,
    system_status: u8,
    previous_state: Io4State,
}

struct Io4AsyncRead {
    read_buf: usize,
    out_nbytes: usize,
    ovl: usize,
}

#[derive(Clone, Copy)]
struct Io4ReadTarget {
    buf: *mut u8,
    nbytes: u32,
    out_nbytes: *mut u32,
    ovl: *mut c_void,
}

impl<O: Io4Ops> Io4Device<O> {
    pub fn new(ops: O, foreground_only: bool) -> Self {
        Self {
            ops,
            foreground_only,
            system_status: 0x02,
            previous_state: Io4State::default(),
        }
    }

    pub fn matches_path(path: &str) -> bool {
        path.eq_ignore_ascii_case(IO4_PATH) || path.to_ascii_lowercase().contains("vid_0ca3")
    }

    pub fn read_report(&mut self) -> [u8; REPORT_LEN] {
        let is_foreground = !self.foreground_only || foreground_matches("teaGfx DirectX Release");
        let state = if self.foreground_only && !is_foreground {
            self.previous_state
        } else {
            let state = self.ops.poll();
            self.previous_state = state;
            state
        };

        let mut report = [0u8; REPORT_LEN];
        report[0] = 1;
        let mut pos = 1;
        write_u16s(&mut report, &mut pos, &state.adcs);
        write_u16s(&mut report, &mut pos, &state.spinners);
        write_u16s(&mut report, &mut pos, &state.chutes);
        write_u16s(&mut report, &mut pos, &state.buttons);
        report[pos] = self.system_status;
        report[pos + 1] = 0;
        report
    }

    pub fn write_report(&mut self, report: &[u8]) -> Result<(), i32> {
        if report.len() != REPORT_LEN || report[0] != 0x10 {
            return Err(-1);
        }

        match report[1] {
            0x01 | 0x02 => {
                self.system_status = 0x30;
                Ok(())
            }
            0x03 => {
                self.system_status = 0;
                Ok(())
            }
            0x04 => self.ops.write_gpio(&report[2..2 + OUT_PAYLOAD_LEN]),
            0x05 => self.ops.write_pwm(&report[2..2 + OUT_PAYLOAD_LEN]),
            0x41 => self.ops.write_unique(&report[2..2 + OUT_PAYLOAD_LEN]),
            _ => Err(-1),
        }
    }
}

pub fn init(api: &Api, config: &Config) {
    let Some(config) = config
        .section::<Io4Config>()
        .filter(|config| config.enabled)
    else {
        return;
    };

    unsafe {
        let Some(fd) = iohook::open_nul_fd() else {
            api.log_info("IO4 emulator skipped: failed to open NUL handle");
            return;
        };

        if let Ok(mut io4_fd) = IO4_FD.lock() {
            *io4_fd = fd;
        }
        if let Ok(mut device) = IO4_DEVICE.lock() {
            *device = Some(Io4Device::new(
                chusan_io4::ChusanIo4Ops,
                config.foreground,
            ));
        }
        iohook::push_handler(io4_irp_handler);
    }
    api.log_info("IO4 emulator initialized");
}

unsafe fn io4_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        let matched = matches_irp_path(irp);
        if matched {
            if let Some(api) = crate::util::api::API.get() {
                api.log_info("IO4: OPEN matched $io4\\vid_0ca3");
            }
        }
        if matched {
            irp.fd = io4_fd();
            return 1;
        }
        return iohook::invoke_next(irp);
    }

    if irp.fd != io4_fd() {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => 1,
        IrpOp::Read => read_irp(irp),
        IrpOp::Write => write_irp(irp),
        IrpOp::Ioctl => ioctl_irp(irp),
        IrpOp::Fsync | IrpOp::Seek | IrpOp::Open => 1,
    }
}

fn io4_fd() -> HANDLE {
    IO4_FD.lock().map_or(0, |fd| *fd)
}

unsafe fn matches_irp_path(irp: &Irp) -> bool {
    parse_path_a(irp.open_filename_a)
        .or_else(|| parse_path_w(irp.open_filename_w))
        .is_some_and(|path| Io4Device::<chusan_io4::ChusanIo4Ops>::matches_path(&path))
}

unsafe fn read_irp(irp: &mut Irp) -> i32 {
    read_report_target(Io4ReadTarget {
        buf: irp.read_buf,
        nbytes: irp.nbytes,
        out_nbytes: irp.out_nbytes,
        ovl: irp.ovl,
    })
}

unsafe fn read_report_target(target: Io4ReadTarget) -> i32 {
    if target.buf.is_null() || target.nbytes < REPORT_LEN as u32 {
        return 0;
    }
    if !target.ovl.is_null() {
        return submit_async_read(target);
    }
    let Ok(mut device) = IO4_DEVICE.lock() else {
        return 0;
    };
    let Some(device) = device.as_mut() else {
        return 0;
    };
    let report = device.read_report();
    ptr::copy_nonoverlapping(report.as_ptr(), target.buf, REPORT_LEN);
    set_out_nbytes(target.out_nbytes, REPORT_LEN as u32);
    1
}

unsafe fn submit_async_read(target: Io4ReadTarget) -> i32 {
    let ovl = target.ovl.cast::<OVERLAPPED>();
    (*ovl).Internal = STATUS_PENDING;
    let task = Io4AsyncRead {
        read_buf: target.buf as usize,
        out_nbytes: target.out_nbytes as usize,
        ovl: target.ovl as usize,
    };
    match io4_async_sender().send(task) {
        Ok(()) => {
            iohook::set_last_error(ERROR_IO_PENDING);
            0
        }
        Err(_) => 0,
    }
}

fn io4_async_sender() -> &'static Sender<Io4AsyncRead> {
    IO4_ASYNC.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Io4AsyncRead>();
        thread::spawn(move || {
            while let Ok(task) = rx.recv() {
                unsafe { complete_async_read(task) };
            }
        });
        tx
    })
}

unsafe fn complete_async_read(task: Io4AsyncRead) {
    let report = IO4_DEVICE
        .lock()
        .ok()
        .and_then(|mut device| device.as_mut().map(Io4Device::read_report))
        .unwrap_or([0; REPORT_LEN]);
    ptr::copy_nonoverlapping(report.as_ptr(), task.read_buf as *mut u8, REPORT_LEN);
    if task.out_nbytes != 0 {
        *(task.out_nbytes as *mut u32) = REPORT_LEN as u32;
    }
    let ovl = task.ovl as *mut OVERLAPPED;
    (*ovl).InternalHigh = REPORT_LEN;
    let event = (*ovl).hEvent;
    fence(Ordering::SeqCst);
    (*ovl).Internal = STATUS_SUCCESS;
    if event != 0 {
        SetEvent(event);
    }
}

unsafe fn write_irp(irp: &mut Irp) -> i32 {
    if irp.write_buf.is_null() || irp.nbytes < REPORT_LEN as u32 {
        return 0;
    }
    let Ok(mut device) = IO4_DEVICE.lock() else {
        return 0;
    };
    let Some(device) = device.as_mut() else {
        return 0;
    };
    let report = std::slice::from_raw_parts(irp.write_buf, REPORT_LEN);
    if device.write_report(report).is_err() {
        return 0;
    }
    set_out_nbytes(irp.out_nbytes, REPORT_LEN as u32);
    1
}

unsafe fn ioctl_irp(irp: &mut Irp) -> i32 {
    match irp.ioctl {
        IOCTL_HID_GET_MANUFACTURER_STRING => copy_utf16(
            &manufacturer_string_utf16(),
            irp.ioctl_out,
            irp.ioctl_out_nbytes,
            irp.out_nbytes,
        ),
        IOCTL_HID_GET_PRODUCT_STRING => copy_utf16(
            &product_string_utf16(),
            irp.ioctl_out,
            irp.ioctl_out_nbytes,
            irp.out_nbytes,
        ),
        IOCTL_HID_GET_INPUT_REPORT => read_ioctl_report(irp),
        IOCTL_HID_SET_OUTPUT_REPORT => write_ioctl_report(irp),
        _ => 1,
    }
}

unsafe fn read_ioctl_report(irp: &mut Irp) -> i32 {
    read_report_target(Io4ReadTarget {
        buf: irp.ioctl_out.cast::<u8>(),
        nbytes: irp.ioctl_out_nbytes,
        out_nbytes: irp.out_nbytes,
        ovl: irp.ovl,
    })
}

unsafe fn write_ioctl_report(irp: &mut Irp) -> i32 {
    if irp.ioctl_in.is_null() || irp.ioctl_in_nbytes < REPORT_LEN as u32 {
        return 0;
    }
    let Ok(mut device) = IO4_DEVICE.lock() else {
        return 0;
    };
    let Some(device) = device.as_mut() else {
        return 0;
    };
    let report = std::slice::from_raw_parts(irp.ioctl_in.cast::<u8>(), REPORT_LEN);
    if device.write_report(report).is_err() {
        return 0;
    }
    set_out_nbytes(irp.out_nbytes, REPORT_LEN as u32);
    1
}

unsafe fn copy_utf16(text: &[u16], out: *mut c_void, out_size: u32, out_nbytes: *mut u32) -> i32 {
    let bytes = std::slice::from_raw_parts(text.as_ptr().cast::<u8>(), text.len() * 2);
    if out.is_null() || out_size < bytes.len() as u32 {
        return 0;
    }
    ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
    set_out_nbytes(out_nbytes, bytes.len() as u32);
    1
}

fn foreground_matches(expected: &str) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd == 0 {
            return true;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return true;
        }
        let mut title = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        if copied <= 0 {
            return true;
        }
        String::from_utf16_lossy(&title[..copied as usize]) == expected
    }
}

unsafe fn set_out_nbytes(out_nbytes: *mut u32, value: u32) {
    if !out_nbytes.is_null() {
        *out_nbytes = value;
    }
}

unsafe fn parse_path_a(ptr: *const u8) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 && len < 260 {
        len += 1;
    }
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len))
        .ok()
        .map(str::to_owned)
}

unsafe fn parse_path_w(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 && len < 260 {
        len += 1;
    }
    String::from_utf16(std::slice::from_raw_parts(ptr, len)).ok()
}

pub fn manufacturer_string_utf16() -> Vec<u16> {
    "SEGA\0".encode_utf16().collect()
}

pub fn product_string_utf16() -> Vec<u16> {
    "I/O CONTROL BD;15257;01;90;1831;6679A;00;GOUT=14_ADIN=8,E_ROTIN=4_COININ=2_SWIN=2,E_UQ1=41,6\0"
        .encode_utf16()
        .collect()
}

fn write_u16s<const N: usize>(buf: &mut [u8; REPORT_LEN], pos: &mut usize, values: &[u16; N]) {
    for value in values {
        let bytes = value.to_le_bytes();
        buf[*pos] = bytes[0];
        buf[*pos + 1] = bytes[1];
        *pos += 2;
    }
}
