use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::chuniio;
use crate::iohook::uart;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct SliderDeviceConfig => SLIDER_DEVICE_CONFIG_SECTION {
        section: "SliderDevice",
        order: 350,
        default_enabled: true,
        always_enabled: false,
        hidden: false,
        comment: "触摸条设备模拟",
        fields: {}
    }
}

const SYNC: u8 = 0xFF;
const ESC: u8 = 0xFD;
const CMD_AUTO_SCAN: u8 = 0x01;
const CMD_SET_LED: u8 = 0x02;
const CMD_AUTO_SCAN_START: u8 = 0x03;
const CMD_AUTO_SCAN_STOP: u8 = 0x04;
const CMD_RESET: u8 = 0x10;
const CMD_GET_BOARD_INFO: u8 = 0xF0;
const SLIDER_PORT: u32 = 1;
const BOARD_INFO: [u8; 32] = [
    b'1', b'5', b'3', b'3', b'0', b' ', b' ', b' ', 0xA0, b'0', b'6', b'7', b'1', b'2', 0xFF, 0x90,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

static SLIDER: Mutex<Option<SliderDevice>> = Mutex::new(None);
static SLIDER_FD: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub struct SliderDevice {
    rx: Vec<u8>,
    tx: Arc<Mutex<Vec<u8>>>,
}

impl SliderDevice {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), i32> {
        self.rx.extend_from_slice(bytes);

        while let Some(frame) = decode_frame(&mut self.rx)? {
            self.dispatch(&frame)?;
        }

        Ok(())
    }

    pub fn read(&self, out: &mut Vec<u8>) {
        if let Ok(mut tx) = self.tx.lock() {
            out.extend_from_slice(&tx);
            tx.clear();
        }
    }

    fn dispatch(&mut self, frame: &[u8]) -> Result<(), i32> {
        if frame.len() < 3 {
            return Err(-1);
        }

        match frame[1] {
            CMD_RESET => self.push_response(CMD_RESET, &[]),
            CMD_GET_BOARD_INFO => self.push_response(CMD_GET_BOARD_INFO, &BOARD_INFO),
            CMD_SET_LED => {
                if frame.len() >= 4 + 96 {
                    chuniio::slider_set_leds(&frame[4..4 + 96]);
                }
                Ok(())
            }
            CMD_AUTO_SCAN_START => {
                let tx = self.tx.clone();
                chuniio::slider_start(Arc::new(move |pressure| {
                    if let Ok(mut tx) = tx.lock() {
                        let _ = encode_frame_into(&mut tx, CMD_AUTO_SCAN, &pressure);
                    }
                }));
                Ok(())
            }
            CMD_AUTO_SCAN_STOP => {
                chuniio::slider_stop();
                self.push_response(CMD_AUTO_SCAN_STOP, &[])
            }
            _ => Ok(()),
        }
    }

    fn push_response(&self, cmd: u8, payload: &[u8]) -> Result<(), i32> {
        let Ok(mut tx) = self.tx.lock() else {
            return Err(-1);
        };
        encode_frame_into(&mut tx, cmd, payload)
    }
}

#[applechu_macros::config_section(stage = Device, order = 30)]
pub fn init(api: &Api, _config: &SliderDeviceConfig) {
    if chuniio::slider_init().is_ok() {
        if let Ok(mut slider) = SLIDER.lock() {
            *slider = Some(SliderDevice::new());
        }
        unsafe {
            iohook::push_handler(slider_irp_handler);
        }
        api.log_info("Slider emulator initialized");
    }
}

unsafe fn slider_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        if !matches_com_port(irp, SLIDER_PORT) {
            return iohook::invoke_next(irp);
        }
        uart::uart_init(SLIDER_PORT);
        let handle = uart::fake_handle_pub(SLIDER_PORT);
        irp.fd = handle as isize;
        SLIDER_FD.store(handle, Ordering::SeqCst);
        return 1;
    }

    let my_fd = SLIDER_FD.load(Ordering::SeqCst);
    if my_fd == 0 || irp.fd as usize != my_fd {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => {
            SLIDER_FD.store(0, Ordering::SeqCst);
            1
        }
        IrpOp::Read => read_slider(irp),
        IrpOp::Write => write_slider(irp),
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

unsafe fn read_slider(irp: &mut Irp) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.read_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }

    let mut data = Vec::new();
    if let Ok(slider) = SLIDER.lock() {
        if let Some(device) = slider.as_ref() {
            device.read(&mut data);
        }
    }

    let count = data.len().min(irp.nbytes as usize);
    std::ptr::copy_nonoverlapping(data.as_ptr(), irp.read_buf, count);
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = count as u32;
    }
    1
}

unsafe fn write_slider(irp: &mut Irp) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.write_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }

    let bytes = std::slice::from_raw_parts(irp.write_buf, irp.nbytes as usize);
    if contains_auto_scan_stop(bytes) {
        let mut data = Vec::new();
        if let Ok(mut slider) = SLIDER.lock() {
            if let Some(device) = slider.as_mut() {
                device.rx.extend_from_slice(bytes);
                while let Some(frame) = match decode_frame(&mut device.rx) {
                    Ok(frame) => frame,
                    Err(_) => return 0,
                } {
                    if frame.get(1).copied() == Some(CMD_AUTO_SCAN_STOP) {
                        device.read(&mut data);
                    } else if device.dispatch(&frame).is_err() {
                        return 0;
                    }
                }
            }
        }
        chuniio::slider_stop();
        if let Ok(mut slider) = SLIDER.lock() {
            if let Some(device) = slider.as_mut() {
                if device.push_response(CMD_AUTO_SCAN_STOP, &[]).is_err() {
                    return 0;
                }
            }
        }
        if !irp.out_nbytes.is_null() {
            *irp.out_nbytes = irp.nbytes;
        }
        return 1;
    }
    if let Ok(mut slider) = SLIDER.lock() {
        if let Some(device) = slider.as_mut() {
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

fn contains_auto_scan_stop(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|window| window == [SYNC, CMD_AUTO_SCAN_STOP])
}

fn decode_frame(rx: &mut Vec<u8>) -> Result<Option<Vec<u8>>, i32> {
    let Some(sync) = rx.iter().position(|byte| *byte == SYNC) else {
        rx.clear();
        return Ok(None);
    };
    if sync > 0 {
        rx.drain(..sync);
    }

    let mut decoded = Vec::with_capacity(rx.len());
    let mut escape = false;
    for (idx, byte) in rx.iter().copied().enumerate() {
        if idx == 0 {
            decoded.push(byte);
            continue;
        }
        if byte == SYNC {
            return Err(-1);
        }
        if byte == ESC {
            if escape {
                return Err(-1);
            }
            escape = true;
            continue;
        }

        decoded.push(if escape { byte.wrapping_add(1) } else { byte });
        escape = false;

        if decoded.len() >= 4 && decoded.len() == decoded[2] as usize + 4 {
            let checksum = decoded
                .iter()
                .fold(0u8, |sum, value| sum.wrapping_add(*value));
            if checksum != 0 {
                return Err(-1);
            }
            rx.drain(..=idx);
            decoded.pop();
            return Ok(Some(decoded));
        }
    }

    Ok(None)
}

fn encode_frame_into(tx: &mut Vec<u8>, cmd: u8, payload: &[u8]) -> Result<(), i32> {
    let nbytes = u8::try_from(payload.len()).map_err(|_| -1)?;
    let mut raw = Vec::with_capacity(payload.len() + 4);
    raw.extend_from_slice(&[SYNC, cmd, nbytes]);
    raw.extend_from_slice(payload);
    let checksum = raw.iter().fold(0u8, |sum, value| sum.wrapping_add(*value));
    raw.push(0u8.wrapping_sub(checksum));

    tx.push(SYNC);
    for byte in raw.iter().copied().skip(1) {
        if byte == SYNC || byte == ESC {
            tx.push(ESC);
            tx.push(byte.wrapping_sub(1));
        } else {
            tx.push(byte);
        }
    }
    Ok(())
}
