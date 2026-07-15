use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::aime;
use crate::config::Config;
use crate::iohook::uart;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct VfdConfig => VFD_CONFIG_SECTION {
        section: "Vfd",
        order: 390,
        default_enabled: true,
        always_enabled: false,
        hidden: false,
        comment: "VFD 显示板模拟，仅 SP 模式",
        fields: {}
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
const VFD_PORT: u32 = 2;

static VFD: Mutex<Option<VfdDevice>> = Mutex::new(None);
static VFD_FD: AtomicUsize = AtomicUsize::new(0);

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
    tx: Vec<u8>,
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
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), i32> {
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos] == SYNC1 || bytes[pos] == SYNC2 {
                pos += 1;
                if pos >= bytes.len() {
                    break;
                }
                let cmd = bytes[pos];
                pos += 1;
                self.handle_command(cmd, bytes, &mut pos)?;
            } else {
                let start = pos;
                while pos < bytes.len() && bytes[pos] != SYNC1 && bytes[pos] != SYNC2 {
                    pos += 1;
                }
                self.state.text.extend_from_slice(&bytes[start..pos]);
                self.forward_text(&bytes[start..pos]);
            }
        }
        Ok(())
    }

    pub fn drain(&mut self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.tx);
        self.tx.clear();
    }

    fn handle_command(&mut self, cmd: u8, bytes: &[u8], pos: &mut usize) -> Result<(), i32> {
        match cmd {
            CMD_GET_VERSION => {
                if *pos < bytes.len() && !is_sync(bytes[*pos]) {
                    *pos += 1;
                }
                self.tx.extend_from_slice(&[2, b'0', b'1', b'.', b'2', b'0', 1]);
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
                let lines = y1.saturating_sub(y0).saturating_add(1) as usize;
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
        let end_or_sync = bytes
            .get(*pos + 1)
            .is_none_or(|follow| is_sync(*follow));
        if !end_or_sync {
            return false;
        }
        *pos += 1;
        self.state.brightness = next;
        self.forward_state();
        true
    }
}

pub fn init(api: &Api, config: &Config) {
    if !config
        .section::<VfdConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }

    if let Ok(mut device) = VFD.lock() {
        *device = Some(VfdDevice::default());
    }
    unsafe {
        iohook::push_handler(vfd_irp_handler);
    }
    api.log_info("VFD emulator initialized");
}

unsafe fn vfd_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        if !matches_com_port(irp, VFD_PORT) {
            return iohook::invoke_next(irp);
        }
        uart::uart_init(VFD_PORT);
        let handle = uart::fake_handle_pub(VFD_PORT);
        irp.fd = handle as isize;
        VFD_FD.store(handle, Ordering::SeqCst);
        return 1;
    }

    let my_fd = VFD_FD.load(Ordering::SeqCst);
    if my_fd == 0 || irp.fd as usize != my_fd {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => {
            VFD_FD.store(0, Ordering::SeqCst);
            1
        }
        IrpOp::Read => read_vfd(irp),
        IrpOp::Write => write_vfd(irp),
        IrpOp::Ioctl => uart::device_io_control(
            irp.fd as usize,
            irp.ioctl,
            irp.ioctl_in,
            irp.ioctl_in_nbytes,
            irp.ioctl_out,
            irp.ioctl_out_nbytes,
            irp.out_nbytes,
        ),
        IrpOp::Fsync | IrpOp::Seek => 1,
        IrpOp::Open => 1,
    }
}

unsafe fn read_vfd(irp: &mut Irp) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.read_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }

    let mut data = Vec::new();
    if let Ok(mut vfd) = VFD.lock() {
        if let Some(device) = vfd.as_mut() {
            device.drain(&mut data);
        }
    }

    let count = data.len().min(irp.nbytes as usize);
    std::ptr::copy_nonoverlapping(data.as_ptr(), irp.read_buf, count);
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = count as u32;
    }
    1
}

unsafe fn write_vfd(irp: &mut Irp) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.write_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }

    let bytes = std::slice::from_raw_parts(irp.write_buf, irp.nbytes as usize);
    if let Ok(mut vfd) = VFD.lock() {
        if let Some(device) = vfd.as_mut() {
            if device.write(bytes).is_err() {
                return 0;
            }
        }
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = irp.nbytes;
    }
    1
}

unsafe fn matches_com_port(irp: &Irp, port_no: u32) -> bool {
    uart::parse_com_a(irp.open_filename_a).or_else(|| uart::parse_com_w(irp.open_filename_w))
        == Some(port_no)
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
