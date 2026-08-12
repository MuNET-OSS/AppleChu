use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::aime;
use crate::iohook::uart;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct VfdConfig => VFD_CONFIG_SECTION {
        section: "Vfd",
        order: 110,
        default_on: true,
        always_enabled: false,
        hidden: false,
        group: "io",
        comment: "VFD 显示板模拟，仅 SP 模式",
        fields: {
            pub port: u32 = 0,
            key: "portNo",
            advanced: true,
            comment: "VFD 串口号；0 表示使用 SP 默认串口";
            pub utf_conversion: bool = false,
            advanced: true,
            comment: "启用 VFD 文本编码转换日志";
        }
    }
}

const SYNC1: u8 = 0x1B;
const SYNC2: u8 = 0x1F;
const CMD_GET_VERSION: u8 = 0x5B;
const CMD_RESET: u8 = 0x0B;
const CMD_CLEAR_SCREEN: u8 = 0x0C;
const CMD_SET_BRIGHTNESS: u8 = 0x20;
const CMD_SET_SCREEN_ON: u8 = 0x21;
const CMD_SET_H_SCROLL: u8 = 0x22;
const CMD_DRAW_IMAGE: u8 = 0x2E;
const CMD_SET_CURSOR: u8 = 0x30;
const CMD_SET_ENCODING: u8 = 0x32;
const CMD_SET_TEXT_WND: u8 = 0x40;
const CMD_SET_TEXT_SPEED: u8 = 0x41;
const CMD_WRITE_STATIC: u8 = 0x00;
const CMD_WRITE_TEXT: u8 = 0x50;
const CMD_ENABLE_SCROLL: u8 = 0x51;
const CMD_DISABLE_SCROLL: u8 = 0x52;
const CMD_ROTATE: u8 = 0x5D;
const CMD_CREATE_CHAR: u8 = 0xA3;
const CMD_CREATE_CHAR2: u8 = 0xA4;
const VFD_ENC_SHIFT_JIS: u8 = 2;
const VFD_ENC_MAX: u8 = 3;
const VFD_BRIGHTNESS_MAX: u8 = 4;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_FUNCTION: u32 = 1;

static VFD: Mutex<Option<VfdDevice>> = Mutex::new(None);
static VFD_FD: AtomicUsize = AtomicUsize::new(0);
static VFD_PORT: AtomicUsize = AtomicUsize::new(2);

#[derive(Clone)]
pub struct VfdState {
    pub brightness: u8,
    pub screen_on: bool,
    pub encoding: u8,
    pub text_speed: u8,
    pub scroll_enabled: bool,
    pub h_scroll: u16,
    pub cursor_x: u16,
    pub cursor_y: u8,
    pub wnd_x0: u16,
    pub wnd_y0: u8,
    pub wnd_x1: u16,
    pub wnd_y1: u8,
    pub rotate: u8,
    pub clear_seq: u32,
    pub text: Vec<u8>,
}

impl VfdState {
    fn to_aimeio(&self) -> aime::external::AimeIoVfdState {
        aime::external::AimeIoVfdState {
            encoding: self.encoding,
            text_speed: self.text_speed,
            scroll_enabled: u8::from(self.scroll_enabled),
            h_scroll: self.h_scroll,
            cursor_x: self.cursor_x,
            cursor_y: self.cursor_y,
            wnd_x0: self.wnd_x0,
            wnd_y0: self.wnd_y0,
            wnd_x1: self.wnd_x1,
            wnd_y1: self.wnd_y1,
            rotate: self.rotate,
            brightness: self.brightness,
            screen_on: u8::from(self.screen_on),
            clear_seq: self.clear_seq,
        }
    }
}

#[derive(Default)]
pub struct VfdDevice {
    state: VfdState,
}

impl Default for VfdState {
    fn default() -> Self {
        Self {
            encoding: VFD_ENC_SHIFT_JIS,
            ..Self::empty()
        }
    }
}

impl VfdState {
    fn empty() -> Self {
        Self {
            brightness: 0,
            screen_on: false,
            encoding: 0,
            text_speed: 0,
            scroll_enabled: false,
            h_scroll: 0,
            cursor_x: 0,
            cursor_y: 0,
            wnd_x0: 0,
            wnd_y0: 0,
            wnd_x1: 0,
            wnd_y1: 0,
            rotate: 0,
            clear_seq: 0,
            text: Vec::new(),
        }
    }
}

impl VfdDevice {
    pub fn process(&mut self, bytes: &mut Vec<u8>, readable: &mut Vec<u8>) -> Result<(), i32> {
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos] == SYNC1 || bytes[pos] == SYNC2 {
                pos += 1;
                if pos >= bytes.len() {
                    break;
                }
                let cmd = bytes[pos];
                pos += 1;
                self.handle_command(cmd, bytes, &mut pos, readable)?;
            } else {
                let start = pos;
                while pos < bytes.len() && bytes[pos] != SYNC1 && bytes[pos] != SYNC2 {
                    pos += 1;
                }
                self.state.text.extend_from_slice(&bytes[start..pos]);
                self.forward_text(&bytes[start..pos]);
            }
        }
        bytes.clear();
        Ok(())
    }

    fn handle_command(
        &mut self,
        cmd: u8,
        bytes: &[u8],
        pos: &mut usize,
        readable: &mut Vec<u8>,
    ) -> Result<(), i32> {
        match cmd {
            CMD_GET_VERSION => {
                if *pos < bytes.len() && !is_sync(bytes[*pos]) {
                    *pos += 1;
                }
                readable.extend_from_slice(&[2, b'0', b'1', b'.', b'2', b'0', 1]);
            }
            CMD_RESET => {
                self.state = VfdState::default();
                self.forward_state();
            }
            CMD_CLEAR_SCREEN => {
                if self.consume_compat_brightness(bytes, pos) {
                    return Ok(());
                }
                self.state.text.clear();
                self.state.clear_seq = self.state.clear_seq.wrapping_add(1);
                self.forward_state();
            }
            CMD_SET_BRIGHTNESS => {
                let brightness = take_u8(bytes, pos).unwrap_or(0);
                if brightness <= VFD_BRIGHTNESS_MAX {
                    self.state.brightness = brightness;
                }
                self.forward_state();
            }
            CMD_SET_SCREEN_ON => {
                let screen_on = take_u8(bytes, pos).unwrap_or(0);
                if screen_on <= 1 {
                    self.state.screen_on = screen_on != 0;
                }
                self.forward_state();
            }
            CMD_SET_H_SCROLL => {
                self.state.h_scroll = take_be_u16(bytes, pos).unwrap_or(0);
                self.forward_state();
            }
            CMD_DRAW_IMAGE => {
                let _x0 = take_be_u16(bytes, pos).unwrap_or(0);
                let y0 = take_u8(bytes, pos).unwrap_or(0);
                let width = take_be_u16(bytes, pos).unwrap_or(0);
                let y1 = take_u8(bytes, pos).unwrap_or(0);
                let lines = if y1 >= y0 {
                    usize::from(y1 - y0 + 1)
                } else {
                    0
                };
                let payload = width as usize * lines * 8;
                *pos = (*pos + payload).min(bytes.len());
            }
            CMD_SET_CURSOR => {
                self.state.cursor_x = take_be_u16(bytes, pos).unwrap_or(0);
                self.state.cursor_y = take_u8(bytes, pos).unwrap_or(0);
                self.forward_state();
            }
            CMD_SET_ENCODING => {
                let encoding = take_u8(bytes, pos).unwrap_or(VFD_ENC_SHIFT_JIS);
                if encoding <= VFD_ENC_MAX {
                    self.state.encoding = encoding;
                }
                self.forward_state();
            }
            CMD_SET_TEXT_WND => {
                let x0 = take_be_u16(bytes, pos).unwrap_or(0);
                let y0 = take_u8(bytes, pos).unwrap_or(0);
                let width = take_be_u16(bytes, pos).unwrap_or(0);
                let height = take_u8(bytes, pos).unwrap_or(0);
                self.state.wnd_x0 = x0;
                self.state.wnd_y0 = y0;
                self.state.wnd_x1 = x0.wrapping_add(width);
                self.state.wnd_y1 = y0.wrapping_add(height);
                self.forward_state();
            }
            CMD_SET_TEXT_SPEED => {
                self.state.text_speed = take_u8(bytes, pos).unwrap_or(0);
                self.forward_state();
            }
            CMD_WRITE_STATIC => {
                let text = take_until_sync(bytes, pos);
                self.state.text.extend_from_slice(text);
                self.forward_text(text);
            }
            CMD_WRITE_TEXT => {
                if let Some(len) = take_u8(bytes, pos) {
                    let end = (*pos + len as usize).min(bytes.len());
                    self.state.text.extend_from_slice(&bytes[*pos..end]);
                    self.forward_text(&bytes[*pos..end]);
                    *pos = end;
                }
            }
            CMD_ENABLE_SCROLL => {
                self.state.scroll_enabled = true;
                self.forward_state();
            }
            CMD_DISABLE_SCROLL => {
                self.state.scroll_enabled = false;
                self.forward_state();
            }
            CMD_ROTATE => {
                self.state.rotate = take_u8(bytes, pos).unwrap_or(0);
                self.forward_state();
            }
            CMD_CREATE_CHAR => {
                let _kind = take_u8(bytes, pos);
                *pos = (*pos + 32).min(bytes.len());
            }
            CMD_CREATE_CHAR2 => {
                let _kind = take_u8(bytes, pos);
                let _slot = take_u8(bytes, pos);
                *pos = (*pos + 16).min(bytes.len());
            }
            _ => {}
        }
        Ok(())
    }

    fn forward_text(&self, text: &[u8]) {
        if !text.is_empty() {
            aime::vfd_set_text(text, &self.state.to_aimeio());
        }
    }

    fn forward_state(&self) {
        aime::vfd_set_state(&self.state.to_aimeio());
    }

    fn consume_compat_brightness(&mut self, bytes: &[u8], pos: &mut usize) -> bool {
        let Some(&next) = bytes.get(*pos) else {
            return false;
        };
        if is_sync(next) || next > VFD_BRIGHTNESS_MAX {
            return false;
        }
        let end_or_sync = bytes.get(*pos + 1).is_none_or(|follow| is_sync(*follow));
        if !end_or_sync {
            return false;
        }
        *pos += 1;
        self.state.brightness = next;
        self.forward_state();
        true
    }
}

#[applechu_macros::config_section(
    stage = Device,
    order = 40,
    condition = crate::system_config::is_sp_mode
)]
pub fn init(api: &Api, _config: &VfdConfig) {
    let port = if _config.port == 0 { 2 } else { _config.port };
    if let Ok(mut device) = VFD.lock() {
        *device = Some(VfdDevice::default());
        if let Some(device) = device.as_ref() {
            // 安装 VFD 设备时立即发布默认状态
            // 外部 AimeIO 后端依赖该调用清理上一次进程留下的显示状态
            device.forward_state();
        }
    }
    VFD_PORT.store(port as usize, Ordering::Release);
    unsafe {
        iohook::push_handler(vfd_irp_handler);
    }
    api.log_info(&format!("VFD emulator enabled on COM{port}"));
}

unsafe fn vfd_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        if !matches_com_port(irp, vfd_port()) {
            return iohook::invoke_next(irp);
        }
        if VFD_FD.load(Ordering::SeqCst) != 0 {
            return iohook::hresult_from_win32(ERROR_ACCESS_DENIED);
        }
        let Some(fd) = iohook::open_nul_fd() else {
            return iohook::E_FAIL;
        };
        let handle = crate::util::win32::handle_value(fd);
        uart::bind_handle(handle, vfd_port());
        irp.fd = fd;
        VFD_FD.store(handle, Ordering::SeqCst);
        return iohook::S_OK;
    }

    let my_fd = VFD_FD.load(Ordering::SeqCst);
    if my_fd == 0 || crate::util::win32::handle_value(irp.fd) != my_fd {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => {
            VFD_FD.store(0, Ordering::SeqCst);
            uart::unbind_handle(my_fd);
            iohook::invoke_next(irp)
        }
        IrpOp::Read => uart::uart_handle_irp(irp),
        IrpOp::Write => write_vfd(irp),
        IrpOp::Ioctl => uart::device_io_control(
            crate::util::win32::handle_value(irp.fd),
            irp.ioctl,
            irp.ioctl_in,
            irp.ioctl_in_nbytes,
            irp.ioctl_out,
            irp.ioctl_out_nbytes,
            irp.out_nbytes,
        ),
        IrpOp::Fsync => iohook::S_OK,
        IrpOp::Seek | IrpOp::Open => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
    }
}

unsafe fn write_vfd(irp: &mut Irp) -> i32 {
    let hr = uart::uart_handle_irp(irp);
    if hr < 0 {
        return hr;
    }
    let handle = crate::util::win32::handle_value(irp.fd);
    let Some(mut written) = uart::take_written(handle) else {
        return iohook::E_FAIL;
    };
    let mut readable = Vec::new();
    let result = if let Ok(mut vfd) = VFD.lock() {
        if let Some(device) = vfd.as_mut() {
            device.process(&mut written, &mut readable)
        } else {
            Err(iohook::E_FAIL)
        }
    } else {
        Err(iohook::E_FAIL)
    };
    if !uart::restore_written(handle, written) {
        return iohook::E_FAIL;
    }
    if !readable.is_empty() && !uart::push_readable(handle, &readable) {
        return iohook::E_FAIL;
    }
    result.map_or(iohook::E_FAIL, |()| hr)
}

unsafe fn matches_com_port(irp: &Irp, port_no: u32) -> bool {
    uart::parse_com_a(irp.open_filename_a).or_else(|| uart::parse_com_w(irp.open_filename_w))
        == Some(port_no)
}

fn vfd_port() -> u32 {
    VFD_PORT.load(Ordering::Acquire) as u32
}

fn is_sync(byte: u8) -> bool {
    byte == SYNC1 || byte == SYNC2
}

fn take_u8(bytes: &[u8], pos: &mut usize) -> Option<u8> {
    let value = *bytes.get(*pos)?;
    *pos += 1;
    Some(value)
}

fn take_be_u16(bytes: &[u8], pos: &mut usize) -> Option<u16> {
    let hi = u16::from(take_u8(bytes, pos)?);
    let lo = u16::from(take_u8(bytes, pos)?);
    Some((hi << 8) | lo)
}

fn take_until_sync<'a>(bytes: &'a [u8], pos: &mut usize) -> &'a [u8] {
    let start = *pos;
    while *pos < bytes.len() && !is_sync(bytes[*pos]) {
        *pos += 1;
    }
    &bytes[start..*pos]
}
