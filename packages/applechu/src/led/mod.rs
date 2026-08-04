use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::config::Config;
use crate::iohook::uart;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

const SYNC: u8 = 0xE0;
const ESC: u8 = 0xD0;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_FUNCTION: u32 = 1;
const STATUS_OK: u8 = 0x01;
const REPORT_OK: u8 = 0x01;
const REPORT_ERR2: u8 = 0x04;
const LED_DATA_LEN: usize = 198;
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

// 所有架构都使用共享、重叠串口句柄
const EMULATED_OPEN_SHARE: u32 = 3; // FILE_SHARE_READ | FILE_SHARE_WRITE
const EMULATED_OPEN_FLAGS: u32 = 0x4000_0000; // FILE_FLAG_OVERLAPPED

crate::config_section! {
    pub(crate) struct Led15093SectionConfig => LED_15093_CONFIG_SECTION {
        section: "Led15093",
        order: 320,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "15093 LED 控制板模拟",
        fields: {
            pub port0: u32 = 0,
            comment: "第一块控制板串口号；0 表示按机台模式选择";
            pub port1: u32 = 0,
            comment: "第二块控制板串口号；0 表示按机台模式选择";
            pub board_number: String = String::from("15093-06"),
            key: "boardNumber",
            comment: "控制板型号";
            pub chip_number: String = String::from("6710 "),
            key: "chipNumber",
            comment: "应用固件芯片型号";
            pub boot_chip_number: String = String::from("6709 "),
            key: "bootChipNumber",
            comment: "引导固件芯片型号";
            pub fw_ver: u8 = 0x90,
            key: "fwVer",
            comment: "固件版本";
            pub fw_sum: u16 = 0xADF7,
            key: "fwSum",
            comment: "固件校验和";
            pub high_baudrate: bool = false,
            key: "highBaud",
            comment: "使用高波特率";
        }
    }
}

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
static LED_START_STATUS: Mutex<[Option<i32>; 2]> = Mutex::new([None, None]);

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
    led_count: u8,
    fade_depth: u8,
    fade_cycle: u8,
    // 回复暂存区在一次 WriteFile 处理结束后会转移到公共 UART readable 队列
    tx: Vec<u8>,
    led: [u8; LED_DATA_LEN],
    led_bright: [u8; LED_DATA_LEN],
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
            led_count: 66,
            fade_depth: 32,
            fade_cycle: 8,
            tx: Vec::new(),
            led: [0; LED_DATA_LEN],
            led_bright: [0x3F; LED_DATA_LEN],
        }
    }

    pub fn process(&mut self, written: &mut Vec<u8>) -> Result<(), i32> {
        while let Some(frame) = decode_frame(written)? {
            let result = self.dispatch(&frame);
            self.status_code = STATUS_OK;
            self.report_code = REPORT_OK;
            self.board_status = [0, 0, 0, 1];
            // 命令错误只记录日志，随后继续解析并让本次 WriteFile 成功
            // 只有解帧错误才会作为串口写入错误返回给 AM Daemon
            if let Err(status) = result {
                if let Some(api) = crate::util::api::API.get() {
                    api.log_warn(&format!("LED board command failed: {status:#010x}"));
                }
            }
        }
        Ok(())
    }

    pub fn take_response(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.tx)
    }

    fn dispatch(&mut self, frame: &[u8]) -> Result<(), i32> {
        if frame.len() < 5 {
            return Err(-1);
        }
        let destination = frame[1];
        // 单节点设备处理广播地址和有效节点地址
        if destination > 8 {
            return Ok(());
        }
        let cmd = frame[4];
        let payload = &frame[5..];
        match cmd {
            CMD_RESET => {
                self.enable_bootloader = false;
                self.enable_response = true;
                self.respond(cmd, &[])
            }

            CMD_SET_ID => self.set_id(destination, payload),

            CMD_CLEAR_ID => {
                self.board_addr = 0;
                // CLEAR_ID 不受普通命令回复开关影响
                self.respond(cmd, &[])
            }

            CMD_UPDATE_LED => self.respond_if_enabled(cmd, &[]),
            CMD_SET_MAX_BRIGHT => {
                self.copy_led_data(payload, true)?;
                self.respond_if_enabled(cmd, &[])
            }
            CMD_SET_LED | CMD_SET_FADE_LED => {
                self.copy_led_data(payload, false)?;
                self.respond_if_enabled(cmd, &[])
            }
            CMD_SET_FADE_LEVEL => {
                if payload.len() >= 2 {
                    self.fade_depth = payload[0];
                    self.fade_cycle = payload[1];
                }
                self.respond_if_enabled(cmd, &[])
            }
            CMD_SET_FADE_SHIFT => self.respond_if_enabled(cmd, &[]),

            CMD_SET_TIMEOUT => {
                let count = if payload.len() >= 2 {
                    [payload[1], payload[0]]
                } else {
                    [0, 0]
                };
                // SET_TIMEOUT 始终返回当前计数
                self.respond(cmd, &count)
            }

            CMD_SET_DISABLE_RESPONSE => {
                if let Some(sw) = payload.first().copied() {
                    self.enable_response = sw == 0;
                }
                self.respond(cmd, &[(!self.enable_response) as u8])
            }

            CMD_SET_AUTO_SHIFT => {
                let count = payload.first().copied().unwrap_or(0);
                self.led_count = count;
                self.respond_if_enabled(cmd, &[count])
            }

            CMD_SET_IMM_LED => {
                self.copy_led_data(payload, false)?;
                crate::chuniio::led_set_colors(self.board_index, &mut self.led);
                self.respond_if_enabled(cmd, &[])
            }

            CMD_GET_BOARD_STATUS => {
                let len = if self.enable_bootloader { 3 } else { 4 };
                if payload.first().copied().unwrap_or(0) != 0 {
                    self.board_status = [0, 0, 0, 1];
                }
                // 组装回复前处理 clear 标志
                let status = self.board_status[..len].to_vec();
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

            // 占位固件更新命令始终返回确认
            CMD_FW_UPDATE => self.respond(cmd, &[]),

            _ => {
                self.report_code = REPORT_ERR2;
                Ok(())
            }
        }
    }

    fn set_id(&mut self, destination: u8, payload: &[u8]) -> Result<(), i32> {
        let Some(id) = payload.first().copied() else {
            return Err(iohook::E_FAIL);
        };
        if id == 0 || id > 8 {
            return Err(iohook::hresult_from_win32(20));
        }
        // 广播 SET_ID 只分配给尚未编号的节点；定向命令覆盖当前编号
        if destination != 0 || self.board_addr == 0 {
            self.board_addr = id;
        }
        self.respond(CMD_SET_ID, &[])
    }

    fn copy_led_data(&mut self, payload: &[u8], brightness: bool) -> Result<(), i32> {
        let max = usize::from(self.led_count) * 3;
        if payload.len() > max || payload.len() > LED_DATA_LEN {
            return Err(iohook::E_FAIL);
        }
        let target = if brightness {
            &mut self.led_bright
        } else {
            &mut self.led
        };
        target[..payload.len()].copy_from_slice(payload);
        Ok(())
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
        // 设备结构体按默认 C 对齐布局编码，rx_buf 前有一个填充字节
        // 但编码长度固定为 18 字节，因此线上顺序是填充 0 后跟低字节 0xCC
        info[16] = 0;
        info[17] = 0xCC;
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

#[applechu_macros::config_section(stage = Device, order = 50)]
pub fn init(_api: &Api, config: &Config, section: &Led15093SectionConfig) {
    let is_sp = crate::system_config::is_sp_mode(config);
    let defaults = if is_sp { [20, 21] } else { [2, 3] };
    let ports = [
        if section.port0 == 0 {
            defaults[0]
        } else {
            section.port0
        },
        if section.port1 == 0 {
            defaults[1]
        } else {
            section.port1
        },
    ];

    if let Ok(mut led_ports) = LED_PORTS.lock() {
        *led_ports = ports;
    }
    let led_config = Led15093Config::from_section(section);
    if let Ok(mut boards) = LED_BOARDS.lock() {
        *boards = Some([
            Led15093Device::new(0, 2, 1, led_config.clone()),
            // 每条串口只挂一个节点，因此两块板的节点地址均为 2
            // COM20/COM21（CVT 为 COM2/COM3）负责区分两块物理板
            Led15093Device::new(1, 2, 1, led_config),
        ]);
    }
    if let Ok(mut status) = LED_START_STATUS.lock() {
        *status = [None, None];
    }
    unsafe {
        iohook::push_handler(led_irp_handler);
    }
}

unsafe fn led_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        let Some(index) = open_board_index(irp) else {
            return iohook::invoke_next(irp);
        };
        let port = led_port(index);
        if LED_FDS[index].load(Ordering::SeqCst) != 0 {
            return iohook::hresult_from_win32(ERROR_ACCESS_DENIED);
        }
        let status = start_led_backend(index);
        if status < 0 {
            return status;
        }

        // 将目标改为 NUL 后继续钩子链，以取得有效的重叠 I/O 句柄
        irp.open_filename_w = crate::aime::NUL_FILENAME.as_ptr();
        irp.open_filename_a = ptr::null();
        irp.open_access = 0xC000_0000; // GENERIC_READ | GENERIC_WRITE
        irp.open_share = EMULATED_OPEN_SHARE;
        irp.open_security = ptr::null();
        irp.open_creation = 3; // OPEN_EXISTING
        irp.open_flags = EMULATED_OPEN_FLAGS;
        irp.open_template = ptr::null_mut();

        let result = iohook::invoke_next(irp);
        if result >= 0 {
            let handle = crate::util::win32::handle_value(irp.fd);
            uart::bind_handle(handle, port);
            let high_baudrate = LED_BOARDS
                .lock()
                .ok()
                .and_then(|boards| {
                    boards
                        .as_ref()
                        .map(|boards| boards[index].config.high_baudrate)
                })
                .unwrap_or(false);
            uart::set_baud_rate(handle, if high_baudrate { 460_800 } else { 115_200 });
            LED_FDS[index].store(handle, Ordering::SeqCst);
        } else {
            if let Some(api) = crate::util::api::API.get() {
                api.log_error(&format!(
                    "LED board {} failed to open its serial port: {:#010x}",
                    index, result
                ));
            }
        }
        return result;
    }

    let Some(index) = fd_board_index(crate::util::win32::handle_value(irp.fd)) else {
        return iohook::invoke_next(irp);
    };

    match irp.op {
        IrpOp::Close => {
            LED_FDS[index].store(0, Ordering::SeqCst);
            uart::unbind_handle(crate::util::win32::handle_value(irp.fd));
            iohook::invoke_next(irp)
        }
        IrpOp::Read => uart::uart_handle_irp(irp),
        IrpOp::Write => write_led(irp, index),
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

fn start_led_backend(index: usize) -> i32 {
    let Ok(mut statuses) = LED_START_STATUS.lock() else {
        return iohook::E_FAIL;
    };
    if let Some(status) = statuses[index] {
        return status;
    }

    if let Some(api) = crate::util::api::API.get() {
        api.log_info(&format!("LED board {index} backend starting"));
    }
    let status = crate::chuniio::led_init().map_or_else(|status| status, |()| iohook::S_OK);
    statuses[index] = Some(status);
    if let Some(api) = crate::util::api::API.get() {
        if status < 0 {
            api.log_error(&format!(
                "LED board {index} backend failed to start: {status:#010x}"
            ));
        }
    }
    status
}

unsafe fn write_led(irp: &mut Irp, index: usize) -> i32 {
    let hr = uart::uart_handle_irp(irp);
    if hr < 0 {
        return hr;
    }
    let handle = crate::util::win32::handle_value(irp.fd);
    let Some(mut written) = uart::take_written(handle) else {
        return iohook::E_FAIL;
    };
    let mut response = Vec::new();
    let result = if let Ok(mut boards) = LED_BOARDS.lock() {
        if let Some(devices) = boards.as_mut() {
            let result = devices[index].process(&mut written);
            response = devices[index].take_response();
            result
        } else {
            Err(iohook::E_FAIL)
        }
    } else {
        Err(iohook::E_FAIL)
    };
    if !uart::restore_written(handle, written) {
        return iohook::E_FAIL;
    }
    if !response.is_empty() {
        // 按句柄写入是正常路径；句柄在设备反复开关期间若已解绑，按串口号补投递，避免丢失板卡回复
        let queued = uart::push_readable(handle, &response)
            || uart::push_readable_port(led_port(index), &response);
        if !queued {
            return iohook::E_FAIL;
        }
    }
    result.map_or(iohook::E_FAIL, |()| hr)
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
    fn from_section(config: &Led15093SectionConfig) -> Self {
        Self {
            board_number: fixed_ascii(&config.board_number),
            chip_number: fixed_ascii(&config.chip_number),
            boot_chip_number: fixed_ascii(&config.boot_chip_number),
            fw_ver: config.fw_ver,
            fw_sum: config.fw_sum,
            high_baudrate: config.high_baudrate,
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

fn fd_board_index(fd: usize) -> Option<usize> {
    LED_FDS
        .iter()
        .position(|stored| fd != 0 && stored.load(Ordering::SeqCst) == fd)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Led15093Config {
        Led15093Config {
            board_number: *b"15093-06",
            chip_number: *b"6710 ",
            boot_chip_number: *b"6709 ",
            fw_ver: 0x90,
            fw_sum: 0xADF7,
            high_baudrate: false,
        }
    }

    #[test]
    fn two_serial_boards_use_the_same_node_address() {
        let config = test_config();
        let board0 = Led15093Device::new(0, 2, 1, config.clone());
        let board1 = Led15093Device::new(1, 2, 1, config);

        assert_eq!(board0.board_addr, 2);
        assert_eq!(board1.board_addr, 2);
    }

    #[test]
    fn board_info_matches_expected_c_struct_layout() {
        let mut board = Led15093Device::new(0, 2, 1, test_config());

        assert_eq!(
            board.board_info(),
            [
                b'1', b'5', b'0', b'9', b'3', b'-', b'0', b'6', 0x0A, b'6', b'7', b'1', b'0', b' ',
                0xFF, 0x90, 0x00, 0xCC,
            ]
        );

        board
            .dispatch(&[SYNC, 2, 1, 1, CMD_GET_BOARD_INFO])
            .unwrap();
        assert_eq!(
            board.take_response(),
            [
                SYNC,
                1,
                2,
                0x15,
                STATUS_OK,
                CMD_GET_BOARD_INFO,
                REPORT_OK,
                b'1',
                b'5',
                b'0',
                b'9',
                b'3',
                b'-',
                b'0',
                b'6',
                0x0A,
                b'6',
                b'7',
                b'1',
                b'0',
                b' ',
                0xFF,
                0x90,
                0x00,
                0xCC,
                0xF2,
            ]
        );
    }
}
