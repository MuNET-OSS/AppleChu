pub mod chusan_io4;

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{fence, Ordering};
use std::sync::{Condvar, Mutex, Once, OnceLock};
use std::thread;

use once_cell::sync::Lazy;
use windows_sys::Win32::System::Threading::SetEvent;
use windows_sys::Win32::System::IO::OVERLAPPED;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
};

use crate::config::Config;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct Io4Config => IO4_CONFIG_SECTION {
        section: "Io4",
        order: 60,
        default_on: true,
        always_enabled: false,
        hidden: false,
        group: "io",
        comment: "IO4 USB HID 模拟",
        fields: {
            pub foreground: bool = true,
            advanced: true,
            comment: "仅在游戏窗口位于前台时读取输入";
            pub test: i32 = 0x70,
            comment: "测试按钮虚拟键码";
            pub service: i32 = 0x71,
            comment: "服务按钮虚拟键码";
            pub coin: i32 = 0x72,
            comment: "投币按钮虚拟键码";
            pub ir: i32 = 0x20,
            advanced: true,
            comment: "红外模拟虚拟键码";
            pub air1: i32 = b'4' as i32,
            advanced: true,
            comment: "第 1 组红外传感器按键";
            pub air2: i32 = b'5' as i32,
            advanced: true;
            pub air3: i32 = b'6' as i32,
            advanced: true;
            pub air4: i32 = b'7' as i32,
            advanced: true;
            pub air5: i32 = b'8' as i32,
            advanced: true;
            pub air6: i32 = b'9' as i32,
            advanced: true;
            pub cell1: i32 = b'L' as i32,
            advanced: true,
            comment: "触摸条第 1 单元按键";
            pub cell2: i32 = b'L' as i32, advanced: true;
            pub cell3: i32 = b'L' as i32, advanced: true;
            pub cell4: i32 = b'L' as i32, advanced: true;
            pub cell5: i32 = b'K' as i32, advanced: true;
            pub cell6: i32 = b'K' as i32, advanced: true;
            pub cell7: i32 = b'K' as i32, advanced: true;
            pub cell8: i32 = b'K' as i32, advanced: true;
            pub cell9: i32 = b'J' as i32, advanced: true;
            pub cell10: i32 = b'J' as i32, advanced: true;
            pub cell11: i32 = b'J' as i32, advanced: true;
            pub cell12: i32 = b'J' as i32, advanced: true;
            pub cell13: i32 = b'H' as i32, advanced: true;
            pub cell14: i32 = b'H' as i32, advanced: true;
            pub cell15: i32 = b'H' as i32, advanced: true;
            pub cell16: i32 = b'H' as i32, advanced: true;
            pub cell17: i32 = b'G' as i32, advanced: true;
            pub cell18: i32 = b'G' as i32, advanced: true;
            pub cell19: i32 = b'G' as i32, advanced: true;
            pub cell20: i32 = b'G' as i32, advanced: true;
            pub cell21: i32 = b'F' as i32, advanced: true;
            pub cell22: i32 = b'F' as i32, advanced: true;
            pub cell23: i32 = b'F' as i32, advanced: true;
            pub cell24: i32 = b'F' as i32, advanced: true;
            pub cell25: i32 = b'D' as i32, advanced: true;
            pub cell26: i32 = b'D' as i32, advanced: true;
            pub cell27: i32 = b'D' as i32, advanced: true;
            pub cell28: i32 = b'D' as i32, advanced: true;
            pub cell29: i32 = b'S' as i32, advanced: true;
            pub cell30: i32 = b'S' as i32, advanced: true;
            pub cell31: i32 = b'S' as i32, advanced: true;
            pub cell32: i32 = b'S' as i32, advanced: true;
        }
    }
}

pub const BUTTON_TEST: u16 = 1 << 9;
pub const BUTTON_SERVICE: u16 = 1 << 6;
pub const REPORT_LEN: usize = 0x40;
const OUT_PAYLOAD_LEN: usize = 62;
const IO4_PATH: &str = "$io4\\vid_0ca3";
// Windows hidclass.h：FILE_DEVICE_KEYBOARD(0x0b) 的 HID_OUT/HID_IN 控制码
const IOCTL_HID_GET_MANUFACTURER_STRING: u32 = 0x000B_01BA;
const IOCTL_HID_GET_PRODUCT_STRING: u32 = 0x000B_01BE;
const IOCTL_HID_GET_INPUT_REPORT: u32 = 0x000B_01A2;
const IOCTL_HID_SET_OUTPUT_REPORT: u32 = 0x000B_0195;
const STATUS_PENDING: usize = 0x0000_0103;
const STATUS_SUCCESS: usize = 0;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

static IO4_FD: Mutex<usize> = Mutex::new(0);
static IO4_DEVICE: Lazy<Mutex<Option<Io4Device<chusan_io4::ChusanIo4Ops>>>> =
    Lazy::new(|| Mutex::new(None));
static IO4_ASYNC: OnceLock<Io4Async> = OnceLock::new();

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

#[derive(Clone, Copy)]
struct Io4AsyncRead {
    read_buf: usize,
    ovl: usize,
}

/// IO4 同时只保留一个待处理读取
/// 不能使用无界队列：AM Daemon 会把 OVERLAPPED 所有权视为一次性对象，
/// 队列积压后继续访问旧指针会导致自检阶段出现非确定性崩溃
struct Io4Async {
    pending: Mutex<Option<Io4AsyncRead>>,
    pending_cv: Condvar,
    available_cv: Condvar,
    worker_started: Once,
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
            0x01 => {
                log_io4("IO4 communication timeout configured");
                self.system_status = 0x30;
                Ok(())
            }
            0x02 => {
                log_io4("IO4 sampling count configured");
                self.system_status = 0x30;
                Ok(())
            }
            0x03 => {
                log_io4("IO4 board status cleared");
                self.system_status = 0;
                Ok(())
            }
            0x04 => self.ops.write_gpio(&report[2..2 + OUT_PAYLOAD_LEN]),
            0x05 => self.ops.write_pwm(&report[2..2 + OUT_PAYLOAD_LEN]),
            0x41 => self.ops.write_unique(&report[2..2 + OUT_PAYLOAD_LEN]),
            0x85 => {
                log_io4("IO4 firmware update command is unsupported");
                Err(-1)
            }
            command => {
                log_io4(&format!("IO4 received unknown command {command:02x}"));
                Err(-1)
            }
        }
    }
}

#[applechu_macros::config_section(stage = Device, order = 20)]
pub fn init(api: &Api, root: &Config, config: &Io4Config) -> Result<(), String> {
    api.log_info("IO4 backend starting");
    if let Err(status) = crate::chuniio::jvs_init(root) {
        api.log_error(&format!("IO4 backend failed to start: {status:#010x}"));
        return Err(format!("ChuniIo JVS backend failed ({status:#010x})"));
    }
    if !iohook::setupapi::add_phantom_hid(IO4_PATH) {
        return Err("failed to register SetupAPI HID interface".to_owned());
    }
    unsafe {
        let Some(fd) = iohook::open_nul_fd() else {
            return Err("failed to open NUL handle".to_owned());
        };

        if let Ok(mut io4_fd) = IO4_FD.lock() {
            *io4_fd = crate::util::win32::handle_value(fd);
        }
        if let Ok(mut device) = IO4_DEVICE.lock() {
            *device = Some(Io4Device::new(chusan_io4::ChusanIo4Ops, config.foreground));
        }
        if !iohook::push_handler(io4_irp_handler) {
            return Err("I/O handler table is full".to_owned());
        }
    }
    Ok(())
}

unsafe fn io4_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        let matched = matches_irp_path(irp);
        if matched {
            log_io4("IO4 device opened");
            irp.fd = crate::util::win32::handle_from_value(io4_fd());
            return iohook::S_OK;
        }
        return iohook::invoke_next(irp);
    }

    if crate::util::win32::handle_value(irp.fd) != io4_fd() {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => {
            log_io4("IO4 device closed");
            iohook::S_OK
        }
        IrpOp::Read => read_irp(irp),
        IrpOp::Write => write_irp(irp),
        IrpOp::Ioctl => ioctl_irp(irp),
        IrpOp::Fsync | IrpOp::Seek | IrpOp::Open => {
            iohook::hresult_from_win32(ERROR_INVALID_FUNCTION)
        }
    }
}

fn io4_fd() -> usize {
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
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    if !target.ovl.is_null() {
        return submit_async_read(target);
    }
    let Ok(mut device) = IO4_DEVICE.lock() else {
        return iohook::E_FAIL;
    };
    let Some(device) = device.as_mut() else {
        return iohook::E_FAIL;
    };
    let report = device.read_report();
    ptr::copy_nonoverlapping(report.as_ptr(), target.buf, REPORT_LEN);
    set_out_nbytes(target.out_nbytes, REPORT_LEN as u32);
    iohook::S_OK
}

unsafe fn submit_async_read(target: Io4ReadTarget) -> i32 {
    let ovl = target.ovl.cast::<OVERLAPPED>();
    (*ovl).Internal = STATUS_PENDING;
    let task = Io4AsyncRead {
        read_buf: target.buf as usize,
        ovl: target.ovl as usize,
    };
    io4_async().submit(task)
}

fn io4_async() -> &'static Io4Async {
    IO4_ASYNC.get_or_init(|| Io4Async {
        pending: Mutex::new(None),
        pending_cv: Condvar::new(),
        available_cv: Condvar::new(),
        worker_started: Once::new(),
    })
}

impl Io4Async {
    fn submit(&self, task: Io4AsyncRead) -> i32 {
        self.worker_started.call_once(|| {
            let worker = self as *const Io4Async as usize;
            thread::spawn(move || unsafe { io4_async_worker(worker as *const Io4Async) });
        });
        let Ok(mut pending) = self.pending.lock() else {
            return iohook::E_FAIL;
        };
        while pending.is_some() {
            pending = match self.available_cv.wait(pending) {
                Ok(pending) => pending,
                Err(_) => return iohook::E_FAIL,
            };
        }
        *pending = Some(task);
        self.pending_cv.notify_one();
        iohook::hresult_from_win32(iohook::ERROR_IO_PENDING)
    }
}

unsafe fn io4_async_worker(async_read: *const Io4Async) {
    // `IO4_ASYNC` 永久持有该对象，工作线程只在进程退出时结束
    let async_read = &*async_read;
    loop {
        let task = {
            let Ok(mut pending) = async_read.pending.lock() else {
                return;
            };
            while pending.is_none() {
                pending = match async_read.pending_cv.wait(pending) {
                    Ok(pending) => pending,
                    Err(_) => return,
                };
            }
            // 工作线程复制任务后立即释放提交槽位，再执行设备轮询
            let task = match pending.take() {
                Some(task) => task,
                None => unreachable!("pending IO4 task disappeared"),
            };
            async_read.available_cv.notify_one();
            task
        };
        complete_async_read(task);
    }
}

unsafe fn complete_async_read(task: Io4AsyncRead) {
    let report = IO4_DEVICE
        .lock()
        .ok()
        .and_then(|mut device| device.as_mut().map(Io4Device::read_report))
        .unwrap_or([0; REPORT_LEN]);
    ptr::copy_nonoverlapping(report.as_ptr(), task.read_buf as *mut u8, REPORT_LEN);
    let ovl = task.ovl as *mut OVERLAPPED;
    (*ovl).InternalHigh = REPORT_LEN;
    let event = (*ovl).hEvent;
    fence(Ordering::SeqCst);
    (*ovl).Internal = STATUS_SUCCESS;
    if !event.is_null() {
        SetEvent(event);
    }
}

unsafe fn write_irp(irp: &mut Irp) -> i32 {
    if irp.write_buf.is_null() || irp.nbytes < REPORT_LEN as u32 {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    let Ok(mut device) = IO4_DEVICE.lock() else {
        return iohook::E_FAIL;
    };
    let Some(device) = device.as_mut() else {
        return iohook::E_FAIL;
    };
    let report = std::slice::from_raw_parts(irp.write_buf, REPORT_LEN);
    if device.write_report(report).is_err() {
        return iohook::E_FAIL;
    }
    set_out_nbytes(irp.out_nbytes, REPORT_LEN as u32);
    iohook::S_OK
}

unsafe fn ioctl_irp(irp: &mut Irp) -> i32 {
    match irp.ioctl {
        IOCTL_HID_GET_MANUFACTURER_STRING => {
            log_io4("IO4 manufacturer string requested");
            copy_utf16(
                &manufacturer_string_utf16(),
                irp.ioctl_out,
                irp.ioctl_out_nbytes,
                irp.out_nbytes,
            )
        }
        IOCTL_HID_GET_PRODUCT_STRING => {
            log_io4("IO4 product string requested");
            copy_utf16(
                &product_string_utf16(),
                irp.ioctl_out,
                irp.ioctl_out_nbytes,
                irp.out_nbytes,
            )
        }
        IOCTL_HID_GET_INPUT_REPORT => {
            log_io4("IO4 control read requested");
            read_ioctl_report(irp)
        }
        IOCTL_HID_SET_OUTPUT_REPORT => {
            log_io4("IO4 control write requested");
            write_ioctl_report(irp)
        }
        code => {
            log_io4(&format!(
                "IO4 received unknown IOCTL {code:#08x}: input={} bytes, output={} bytes",
                irp.ioctl_in_nbytes, irp.ioctl_out_nbytes
            ));
            iohook::hresult_from_win32(ERROR_INVALID_FUNCTION)
        }
    }
}

fn log_io4(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
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
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    let Ok(mut device) = IO4_DEVICE.lock() else {
        return iohook::E_FAIL;
    };
    let Some(device) = device.as_mut() else {
        return iohook::E_FAIL;
    };
    let report = std::slice::from_raw_parts(irp.ioctl_in.cast::<u8>(), REPORT_LEN);
    if device.write_report(report).is_err() {
        return iohook::E_FAIL;
    }
    set_out_nbytes(irp.out_nbytes, REPORT_LEN as u32);
    iohook::S_OK
}

unsafe fn copy_utf16(text: &[u16], out: *mut c_void, out_size: u32, out_nbytes: *mut u32) -> i32 {
    let bytes = std::slice::from_raw_parts(text.as_ptr().cast::<u8>(), text.len() * 2);
    if out.is_null() || out_size < bytes.len() as u32 {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    ptr::copy_nonoverlapping(bytes.as_ptr(), out.cast::<u8>(), bytes.len());
    set_out_nbytes(out_nbytes, bytes.len() as u32);
    iohook::S_OK
}

fn foreground_matches(expected: &str) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
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
