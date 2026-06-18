use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::config::Config;
use crate::iohook::uart;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

const SYNC: u8 = 0xE0;
const ESC: u8 = 0xD0;
const STATUS_OK: u8 = 0x01;
const REPORT_OK: u8 = 0x01;
const CMD_RESET: u8 = 0x10;
const CMD_SET_TIMEOUT: u8 = 0x11;
const CMD_SET_DISABLE_RESPONSE: u8 = 0x14;
const CMD_SET_ID: u8 = 0x18;
const CMD_CLEAR_ID: u8 = 0x19;
const CMD_SET_MAX_BRIGHT: u8 = 0x3F;
const CMD_UPDATE_LED: u8 = 0x80;
const CMD_SET_LED: u8 = 0x81;
const CMD_SET_IMM_LED: u8 = 0x82;
const CMD_SET_FADE_LED: u8 = 0x83;
const CMD_SET_FADE_LEVEL: u8 = 0x84;
const CMD_SET_FADE_SHIFT: u8 = 0x85;
const CMD_SET_AUTO_SHIFT: u8 = 0x86;
const CMD_GET_BOARD_INFO: u8 = 0xF0;
const CMD_GET_BOARD_STATUS: u8 = 0xF1;
const CMD_GET_FW_SUM: u8 = 0xF2;
const CMD_GET_PROTOCOL_VER: u8 = 0xF3;
const CMD_SET_BOOTMODE: u8 = 0xFD;
const CMD_FW_UPDATE: u8 = 0xFE;

#[derive(Clone)]
struct Led15093Config {
    board_number: [u8; 8],
    chip_number: [u8; 5],
    boot_chip_number: [u8; 5],
    fw_ver: u8,
    fw_sum: u16,
    high_baudrate: bool,
}
static LED_PORTS: Mutex<[u32; 2]> = Mutex::new([2, 3]);

static LED_BOARDS: Mutex<Option<[Led15093Device; 2]>> = Mutex::new(None);
static LED_FDS: [AtomicUsize; 2] = [AtomicUsize::new(0), AtomicUsize::new(0)];

pub struct Led15093Device {
    board_index: u8,
    board_addr: u8,
    host_addr: u8,
    config: Led15093Config,
    enable_bootloader: bool,
    enable_response: bool,
    board_status: [u8; 4],
    status_code: u8,
    report_code: u8,
    rx: Vec<u8>,
    tx: Vec<u8>,
    led: Vec<u8>,
}

impl Led15093Device {
    fn new(board_index: u8, board_addr: u8, host_addr: u8, config: Led15093Config) -> Self {
        Self {
            board_index,
            board_addr,
            host_addr,
            config,
            enable_bootloader: false,
            enable_response: true,
            board_status: [0, 0, 0, 1],
            status_code: STATUS_OK,
            report_code: REPORT_OK,
            rx: Vec::new(),
            tx: Vec::new(),
            led: Vec::new(),
        }
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<(), i32> {
        self.rx.extend_from_slice(bytes);
        while let Some(frame) = decode_frame(&mut self.rx)? {
            self.dispatch(&frame)?;
        }
        Ok(())
    }

    pub fn drain(&mut self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.tx);
        self.tx.clear();
    }

    fn dispatch(&mut self, frame: &[u8]) -> Result<(), i32> {
        if frame.len() < 5 {
            return Err(-1);
        }
        let cmd = frame[4];
        let payload = &frame[5..];
        match cmd {
            CMD_RESET => {
                self.enable_bootloader = false;
                self.enable_response = true;
                self.respond(cmd, &[])
            }

            CMD_SET_ID => {
                if let Some(id) = payload.first().copied() {
                    self.board_addr = id;
                }
                self.respond_if_enabled(cmd, &[])
            }

            CMD_CLEAR_ID => {
                self.board_addr = 0;
                self.respond_if_enabled(cmd, &[])
            }

            CMD_UPDATE_LED | CMD_SET_MAX_BRIGHT | CMD_SET_LED | CMD_SET_FADE_LED
            | CMD_SET_FADE_LEVEL | CMD_SET_FADE_SHIFT => self.respond_if_enabled(cmd, &[]),

            CMD_SET_TIMEOUT => {
                let count = if payload.len() >= 2 {
                    [payload[0], payload[1]]
                } else {
                    [0, 0]
                };
                self.respond_if_enabled(cmd, &count)
            }

            CMD_SET_DISABLE_RESPONSE => {
                if let Some(sw) = payload.first().copied() {
                    self.enable_response = sw == 0;
                }
                self.respond(cmd, &[(!self.enable_response) as u8])
            }

            CMD_SET_AUTO_SHIFT => {
                let count = payload.first().copied().unwrap_or(0);
                let target = payload.get(1).copied().unwrap_or(0);
                self.respond_if_enabled(cmd, &[count, target])
            }

            CMD_SET_IMM_LED => {
                self.led.clear();
                self.led.extend_from_slice(payload);
                crate::chuniio::led_set_colors(self.board_index, &mut self.led);
                self.respond_if_enabled(cmd, &[])
            }

            CMD_GET_BOARD_STATUS => {
                let len = if self.enable_bootloader { 3 } else { 4 };
                let status = self.board_status[..len].to_vec();
                if payload.first().copied().unwrap_or(0) != 0 {
                    self.board_status = [0, 0, 0, 1];
                }
                self.respond(cmd, &status)
            }
            CMD_GET_FW_SUM => self.respond(cmd, &self.config.fw_sum.to_be_bytes()),
            CMD_GET_PROTOCOL_VER => {
                if self.enable_bootloader {
                    self.respond(cmd, &[0, 1, 1])
                } else {
                    self.respond(cmd, &[1, 1, 4])
                }
            }
            CMD_GET_BOARD_INFO => {
                if self.is_legacy_board() {
                    self.respond(cmd, &self.legacy_board_info())
                } else {
                    self.respond(cmd, &self.board_info())
                }
            }

            CMD_SET_BOOTMODE => {
                self.enable_bootloader = true;
                self.respond(cmd, &[1])
            }

            CMD_FW_UPDATE => self.respond_if_enabled(cmd, &[]),

            _ => self.respond_if_enabled(cmd, &[]),
        }
    }

    fn board_info(&self) -> [u8; 18] {
        let mut info = [0u8; 18];
        info[..8].copy_from_slice(&self.config.board_number);
        info[8] = 0x0A;
        let chip_number = if self.enable_bootloader {
            &self.config.boot_chip_number
        } else {
            &self.config.chip_number
        };
        info[9..14].copy_from_slice(chip_number);
        info[14] = 0xFF;
        info[15] = self.config.fw_ver;
        info[16] = 0xCC;
        info
    }

    fn legacy_board_info(&self) -> [u8; 10] {
        let mut info = [0u8; 10];
        info[..8].copy_from_slice(&self.config.board_number);
        info[8] = 0xFF;
        info[9] = self.config.fw_ver;
        info
    }

    fn is_legacy_board(&self) -> bool {
        self.config.board_number == fixed_ascii::<8>("15093")
    }

    fn respond_if_enabled(&mut self, cmd: u8, data: &[u8]) -> Result<(), i32> {
        if self.enable_response {
            self.respond(cmd, data)
        } else {
            Ok(())
        }
    }

    fn respond(&mut self, cmd: u8, data: &[u8]) -> Result<(), i32> {
        let mut payload = Vec::with_capacity(data.len() + 3);
        payload.extend_from_slice(&[self.status_code, cmd, self.report_code]);
        payload.extend_from_slice(data);
        encode_frame_into(&mut self.tx, self.host_addr, self.board_addr, &payload)
    }
}

pub fn init(api: &Api, config: &Config, ports: &[u32; 2]) {
    if !config.get_bool_alias(&[("Led15093", "enable"), ("led15093", "enable")], true) {
        return;
    }

    if let Ok(mut led_ports) = LED_PORTS.lock() {
        *led_ports = *ports;
    }
    let led_config = Led15093Config::load(config);
    if let Ok(mut boards) = LED_BOARDS.lock() {
        *boards = Some([
            Led15093Device::new(0, 2, 1, led_config.clone()),
            Led15093Device::new(1, 2, 1, led_config),
        ]);
    }
    unsafe {
        iohook::push_handler(led_irp_handler);
    }
    api.log_info(&format!(
        "LED 15093 emulator initialized (COM{}, COM{})",
        ports[0], ports[1]
    ));
}

unsafe fn led_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        let Some(index) = open_board_index(irp) else {
            return iohook::invoke_next(irp);
        };
        let port = led_port(index);
        uart::uart_init(port);
        let handle = uart::fake_handle_pub(port);
        let high_baudrate = LED_BOARDS
            .lock()
            .ok()
            .and_then(|boards| boards.as_ref().map(|boards| boards[index].config.high_baudrate))
            .unwrap_or(false);
        uart::set_baud_rate(handle, if high_baudrate { 460_800 } else { 115_200 });
        irp.fd = handle as isize;
        LED_FDS[index].store(handle, Ordering::SeqCst);
        return 1;
    }

    let Some(index) = fd_board_index(irp.fd) else {
        return iohook::invoke_next(irp);
    };

    match irp.op {
        IrpOp::Close => {
            LED_FDS[index].store(0, Ordering::SeqCst);
            1
        }
        IrpOp::Read => read_led(irp, index),
        IrpOp::Write => write_led(irp, index),
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

unsafe fn read_led(irp: &mut Irp, index: usize) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.read_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }

    let mut data = Vec::new();
    if let Ok(mut boards) = LED_BOARDS.lock() {
        if let Some(devices) = boards.as_mut() {
            devices[index].drain(&mut data);
        }
    }

    let count = data.len().min(irp.nbytes as usize);
    std::ptr::copy_nonoverlapping(data.as_ptr(), irp.read_buf, count);
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = count as u32;
    }
    1
}

unsafe fn write_led(irp: &mut Irp, index: usize) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.write_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }

    let bytes = std::slice::from_raw_parts(irp.write_buf, irp.nbytes as usize);
    if let Ok(mut boards) = LED_BOARDS.lock() {
        if let Some(devices) = boards.as_mut() {
            if devices[index].write(bytes).is_err() {
                return 0;
            }
        }
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = irp.nbytes;
    }
    1
}

unsafe fn open_board_index(irp: &Irp) -> Option<usize> {
    let port_no = uart::parse_com_a(irp.open_filename_a)
        .or_else(|| uart::parse_com_w(irp.open_filename_w))?;
    let ports = LED_PORTS.lock().ok()?;
    let idx = ports.iter().position(|port| *port == port_no)?;
    drop(ports);
    Some(idx)
}

impl Led15093Config {
    fn load(config: &Config) -> Self {
        Self {
            board_number: fixed_ascii(&config.get_string_alias(
                &[("Led15093", "boardNumber"), ("led15093", "boardNumber")],
                "15093-06",
            )),
            chip_number: fixed_ascii(&config.get_string_alias(
                &[("Led15093", "chipNumber"), ("led15093", "chipNumber")],
                "6710 ",
            )),
            boot_chip_number: fixed_ascii(&config.get_string_alias(
                &[
                    ("Led15093", "bootChipNumber"),
                    ("led15093", "bootChipNumber"),
                ],
                "6709 ",
            )),
            fw_ver: config.get_int_alias(&[("Led15093", "fwVer"), ("led15093", "fwVer")], 0x90)
                as u8,
            fw_sum: config.get_int_alias(&[("Led15093", "fwSum"), ("led15093", "fwSum")], 0xADF7)
                as u16,
            high_baudrate: config.get_bool_alias(
                &[("Led15093", "highBaud"), ("led15093", "highBaud")],
                false,
            ),
        }
    }
}

fn fixed_ascii<const N: usize>(value: &str) -> [u8; N] {
    let mut out = [b' '; N];
    for (idx, byte) in value.bytes().take(N).enumerate() {
        out[idx] = byte;
    }
    out
}

fn fd_board_index(fd: isize) -> Option<usize> {
    LED_FDS
        .iter()
        .position(|stored| fd != 0 && stored.load(Ordering::SeqCst) == fd as usize)
}

fn led_port(index: usize) -> u32 {
    LED_PORTS
        .lock()
        .map_or(if index == 0 { 2 } else { 3 }, |ports| ports[index])
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
        } else if byte == SYNC {
            return Err(-1);
        } else if byte == ESC {
            escape = true;
            continue;
        } else {
            decoded.push(if escape { byte.wrapping_add(1) } else { byte });
            escape = false;
        }

        if decoded.len() >= 5 && decoded.len() == decoded[3] as usize + 5 {
            let checksum = decoded[1..decoded.len() - 1]
                .iter()
                .fold(0u8, |sum, value| sum.wrapping_add(*value));
            if checksum != decoded[decoded.len() - 1] {
                return Err(-1);
            }
            rx.drain(..=idx);
            decoded.pop();
            return Ok(Some(decoded));
        }
    }
    Ok(None)
}

fn encode_frame_into(tx: &mut Vec<u8>, dest: u8, src: u8, payload: &[u8]) -> Result<(), i32> {
    let nbytes = u8::try_from(payload.len()).map_err(|_| -1)?;
    let mut raw = Vec::with_capacity(payload.len() + 5);
    raw.extend_from_slice(&[SYNC, dest, src, nbytes]);
    raw.extend_from_slice(payload);
    raw.push(
        raw[1..]
            .iter()
            .fold(0u8, |sum, value| sum.wrapping_add(*value)),
    );

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
