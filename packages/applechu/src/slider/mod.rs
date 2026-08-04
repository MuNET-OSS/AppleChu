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
        default_on: true,
        always_enabled: false,
        hidden: true,
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
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_FUNCTION: u32 = 1;

static SLIDER: Mutex<Option<SliderDevice>> = Mutex::new(None);
static SLIDER_FD: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub struct SliderDevice;

impl SliderDevice {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process(&mut self, written: &mut Vec<u8>, readable: &mut Vec<u8>) -> Result<(), i32> {
        while let Some(frame) = decode_frame(written)? {
            self.dispatch(&frame, readable)?;
        }
        Ok(())
    }

    fn dispatch(&mut self, frame: &[u8], readable: &mut Vec<u8>) -> Result<(), i32> {
        if frame.len() < 3 {
            return Err(-1);
        }

        let cmd = frame[1];
        match cmd {
            CMD_RESET => {
                log_slider("Slider board reset");
                encode_frame_into(readable, CMD_RESET, &[])
            }
            CMD_GET_BOARD_INFO => {
                log_slider("Slider board firmware information requested");
                encode_frame_into(readable, CMD_GET_BOARD_INFO, &BOARD_INFO)
            }
            CMD_SET_LED => {
                if frame.len() >= 4 + 96 {
                    // 解帧结果已移除校验和：payload[0] 位于 frame[3]，RGB 从 frame[4] 开始
                    chuniio::slider_set_leds(&frame[4..4 + 96]);
                }
                Ok(())
            }
            CMD_AUTO_SCAN_START => {
                log_slider("Slider input started");
                chuniio::slider_start(Arc::new(move |pressure| {
                    let mut response = Vec::with_capacity(36);
                    let _ = encode_frame_into(&mut response, CMD_AUTO_SCAN, &pressure);
                    uart::push_readable_port(SLIDER_PORT, &response);
                }));
                Ok(())
            }
            CMD_AUTO_SCAN_STOP => {
                log_slider("Slider input stopped");
                chuniio::slider_stop();
                encode_frame_into(readable, CMD_AUTO_SCAN_STOP, &[])
            }
            _ => {
                log_slider(&format!("Unhandled command {cmd:02x}"));
                Ok(())
            }
        }
    }
}

#[applechu_macros::config_section(stage = Device, order = 30)]
pub fn init(_api: &Api, _config: &SliderDeviceConfig) {
    if let Ok(mut slider) = SLIDER.lock() {
        *slider = Some(SliderDevice::new());
    }
    unsafe {
        iohook::push_handler(slider_irp_handler);
    }
}

unsafe fn slider_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        if !matches_com_port(irp, SLIDER_PORT) {
            return iohook::invoke_next(irp);
        }
        if SLIDER_FD.load(Ordering::SeqCst) != 0 {
            return iohook::hresult_from_win32(ERROR_ACCESS_DENIED);
        }
        log_slider("Slider backend starting");
        if let Err(status) = chuniio::slider_init() {
            if let Some(api) = crate::util::api::API.get() {
                api.log_error(&format!("Slider backend failed to start: {status:#010x}"));
            }
            return status;
        }
        let Some(fd) = iohook::open_nul_fd() else {
            return iohook::E_FAIL;
        };
        let handle = crate::util::win32::handle_value(fd);
        uart::bind_handle(handle, SLIDER_PORT);
        irp.fd = fd;
        SLIDER_FD.store(handle, Ordering::SeqCst);
        return iohook::S_OK;
    }

    let my_fd = SLIDER_FD.load(Ordering::SeqCst);
    if my_fd == 0 || crate::util::win32::handle_value(irp.fd) != my_fd {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => {
            SLIDER_FD.store(0, Ordering::SeqCst);
            uart::unbind_handle(my_fd);
            iohook::invoke_next(irp)
        }
        IrpOp::Read => uart::uart_handle_irp(irp),
        IrpOp::Write => process_slider_write(irp),
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

unsafe fn process_slider_write(irp: &mut Irp) -> i32 {
    let hr = uart::uart_handle_irp(irp);
    if hr < 0 {
        return hr;
    }
    let handle = crate::util::win32::handle_value(irp.fd);
    let Some(mut written) = uart::take_written(handle) else {
        return iohook::E_FAIL;
    };
    let mut readable = Vec::new();
    let mut result = Ok(());
    if let Ok(mut slider) = SLIDER.lock() {
        if let Some(device) = slider.as_mut() {
            result = device.process(&mut written, &mut readable);
        }
    } else {
        return iohook::E_FAIL;
    }
    if !uart::restore_written(handle, written) {
        return iohook::E_FAIL;
    }
    if !readable.is_empty() {
        if !uart::push_readable(handle, &readable) {
            return iohook::E_FAIL;
        }
    }
    result.map_or(iohook::E_FAIL, |()| hr)
}

fn log_slider(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}

unsafe fn matches_com_port(irp: &Irp, port_no: u32) -> bool {
    uart::parse_com_a(irp.open_filename_a).or_else(|| uart::parse_com_w(irp.open_filename_w))
        == Some(port_no)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_led_rgb_starts_after_the_unknown_payload_byte() {
        let mut encoded = Vec::new();
        let mut payload = [0u8; 97];
        payload[0] = 0x28;
        for (index, byte) in payload[1..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        encode_frame_into(&mut encoded, CMD_SET_LED, &payload).unwrap();

        let decoded = decode_frame(&mut encoded).unwrap().unwrap();
        assert_eq!(decoded[3], 0x28);
        assert_eq!(&decoded[4..4 + 96], &payload[1..]);
    }
}
