use std::ffi::CStr;
use std::fs::File;
use std::io::Read;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::HANDLE;

use applechu::amdaemon::KeychipConfig;
use applechu::iohook::{self, Irp, IrpOp};
use applechu::platform::reg_hook::{self, RegValue, HKEY_LOCAL_MACHINE};
use applechu::util::api::Api;

const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

const NUSEC_IOCTL_PING: u32 = ctl_code(0x22, 0x845, 0, 2);
const NUSEC_IOCTL_GET_PLAY_COUNT: u32 = ctl_code(0x22, 0x854, 0, 3);
const NUSEC_IOCTL_ADD_PLAY_COUNT: u32 = ctl_code(0x22, 0x855, 0, 3);
const NUSEC_IOCTL_ERASE_TRACE_LOG: u32 = ctl_code(0x22, 0x862, 0, 3);
const NUSEC_IOCTL_TD_ERASE_USED: u32 = ctl_code(0x22, 0x863, 0, 3);
const NUSEC_IOCTL_PUT_TRACE_LOG_DATA: u32 = ctl_code(0x22, 0x864, 0, 3);
const NUSEC_IOCTL_GET_TRACE_LOG_DATA: u32 = ctl_code(0x22, 0x865, 0, 3);
const NUSEC_IOCTL_GET_TRACE_LOG_STATE: u32 = ctl_code(0x22, 0x866, 0, 3);
const NUSEC_IOCTL_GET_NVRAM_AVAILABLE: u32 = ctl_code(0x22, 0x867, 0, 3);
const NUSEC_IOCTL_TD_ERASE_ALL: u32 = ctl_code(0x22, 0x869, 0, 3);
const NUSEC_IOCTL_GET_BILLING_CA_CERT: u32 = ctl_code(0x22, 0x871, 0, 3);
const NUSEC_IOCTL_GET_BILLING_PUBKEY: u32 = ctl_code(0x22, 0x872, 0, 3);
const NUSEC_IOCTL_GET_PLAY_LIMIT: u32 = ctl_code(0x22, 0x881, 0, 3);
const NUSEC_IOCTL_PUT_PLAY_LIMIT: u32 = ctl_code(0x22, 0x882, 0, 3);
const NUSEC_IOCTL_GET_NEARFULL: u32 = ctl_code(0x22, 0x883, 0, 3);
const NUSEC_IOCTL_PUT_NEARFULL: u32 = ctl_code(0x22, 0x884, 0, 3);
const NUSEC_IOCTL_GET_NVRAM_GEOMETRY: u32 = ctl_code(0x22, 0x893, 0, 3);

const TRACE_LOG_CAPACITY: u32 = 7_154;
const TRACE_LOG_RECORD_SIZE: usize = 60;
const ERROR_DISK_FULL: u32 = 112;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
static mut NUSEC_FD: HANDLE = std::ptr::null_mut();
static STATE: Mutex<NusecState> = Mutex::new(NusecState {
    nearfull: (1 << 16) + 512,
    play_count: 0,
    play_limit: 1024,
    trace_head: 0,
    trace_tail: 0,
    trace_log: Vec::new(),
    billing_ca: String::new(),
    billing_pub: String::new(),
});

struct NusecState {
    nearfull: u32,
    play_count: u32,
    play_limit: u32,
    trace_head: u32,
    trace_tail: u32,
    trace_log: Vec<[u8; TRACE_LOG_RECORD_SIZE]>,
    billing_ca: String,
    billing_pub: String,
}

#[applechu_macros::config_section(stage = Platform, order = 70)]
pub fn init(api: &Api, config: &KeychipConfig) {
    let keychip = fixed_ascii::<16>(&config.keychip_id);
    let game_id = fixed_ascii::<4>(if config.game_id.is_empty() {
        "SDHD"
    } else {
        &config.game_id
    });
    let platform_id = fixed_ascii::<4>(if config.platform_id.is_empty() {
        "ACA1"
    } else {
        &config.platform_id
    });
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty\\keychip",
        vec![
            RegValue::binary("gameId", game_id),
            RegValue::binary("keychipId", keychip),
            RegValue::dword(
                "modelType",
                platform_id[3]
                    .checked_sub(b'0')
                    .filter(|value| *value <= 9)
                    .map_or(0, u32::from),
            ),
            RegValue::binary("platformId", platform_id[..3].to_vec()),
            RegValue::dword("region", config.region),
            RegValue::binary("serverIpIpv4", subnet_bytes(&config.subnet)),
            RegValue::binary("serverIpIpv6", [0; 16]),
            RegValue::dword("systemFlag", config.system_flag),
        ],
    );

    if let Ok(mut state) = STATE.lock() {
        state.nearfull = (config.billing_type << 16) + 512;
        state.play_count = 0;
        state.play_limit = 1024;
        state.trace_head = 0;
        state.trace_tail = 0;
        state.trace_log = vec![[0; TRACE_LOG_RECORD_SIZE]; TRACE_LOG_CAPACITY as usize];
        state.billing_ca = config.billing_ca.clone();
        state.billing_pub = config.billing_pub.clone();
    }
    unsafe {
        let Some(fd) = iohook::open_nul_fd() else {
            return api.log_warn("Keychip emulator failed to create a device handle");
        };
        NUSEC_FD = fd;
        if !iohook::push_handler(handle_irp) {
            return api.log_warn("Keychip emulator failed to register its device handler");
        }
    }
    api.log_info("Keychip emulator ready");
}

pub(crate) fn subnet_from_config(config: &KeychipConfig) -> u32 {
    parse_ipv4(&config.subnet).unwrap_or(0xC0A8_6400) & 0xFFFF_FF00
}

fn subnet_bytes(value: &str) -> [u8; 4] {
    subnet_from_text(value).to_be_bytes()
}

fn subnet_from_text(value: &str) -> u32 {
    parse_ipv4(value).unwrap_or(0xC0A8_6400) & 0xFFFF_FF00
}

pub(crate) fn parse_ipv4(value: &str) -> Option<u32> {
    let octets = value
        .split('.')
        .map(str::parse::<u8>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (octets.len() == 4).then(|| {
        ((octets[0] as u32) << 24)
            | ((octets[1] as u32) << 16)
            | ((octets[2] as u32) << 8)
            | octets[3] as u32
    })
}

unsafe fn handle_irp(irp: &mut Irp) -> i32 {
    if irp.op != IrpOp::Open && irp.fd != NUSEC_FD {
        return iohook::invoke_next(irp);
    }
    match irp.op {
        IrpOp::Open
            if matches_wide(irp.open_filename_w, "\\??\\FddDriver")
                || matches_ansi(irp.open_filename_a, "\\??\\FddDriver") =>
        {
            irp.fd = NUSEC_FD;
            log_info("Keychip device opened");
            iohook::S_OK
        }
        IrpOp::Open => iohook::invoke_next(irp),
        IrpOp::Close => {
            log_info("Keychip device closed");
            iohook::S_OK
        }
        IrpOp::Ioctl => handle_ioctl(irp),
        _ => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
    }
}

unsafe fn handle_ioctl(irp: &mut Irp) -> i32 {
    let Ok(mut state) = STATE.lock() else {
        return iohook::E_FAIL;
    };
    match irp.ioctl {
        NUSEC_IOCTL_PING => iohook::S_OK,
        NUSEC_IOCTL_ADD_PLAY_COUNT => {
            let Some(delta) = read_input_dword(irp) else {
                return insufficient_input();
            };
            state.play_count = state.play_count.wrapping_add(delta);
            write_dwords(irp, &[state.play_count])
        }
        NUSEC_IOCTL_GET_PLAY_COUNT => write_dwords(irp, &[state.play_count]),
        NUSEC_IOCTL_GET_PLAY_LIMIT => write_dwords(irp, &[state.play_limit]),
        NUSEC_IOCTL_PUT_PLAY_LIMIT => {
            let Some(value) = read_input_dword(irp) else {
                return insufficient_input();
            };
            state.play_limit = value;
            iohook::S_OK
        }
        NUSEC_IOCTL_GET_NEARFULL => write_dwords(irp, &[state.nearfull]),
        NUSEC_IOCTL_PUT_NEARFULL => {
            let Some(value) = read_input_dword(irp) else {
                return insufficient_input();
            };
            state.nearfull = value;
            iohook::S_OK
        }
        NUSEC_IOCTL_TD_ERASE_USED | NUSEC_IOCTL_TD_ERASE_ALL => {
            state.trace_head = 0;
            state.trace_tail = 0;
            iohook::S_OK
        }
        NUSEC_IOCTL_GET_BILLING_CA_CERT => read_billing_file(irp, &state.billing_ca),
        NUSEC_IOCTL_GET_BILLING_PUBKEY => read_billing_file(irp, &state.billing_pub),
        NUSEC_IOCTL_ERASE_TRACE_LOG => {
            let Some(count) = read_input_dword(irp) else {
                return insufficient_input();
            };
            let available = state.trace_head.wrapping_sub(state.trace_tail);
            state.trace_tail = state.trace_tail.wrapping_add(count.max(available));
            iohook::S_OK
        }
        NUSEC_IOCTL_PUT_TRACE_LOG_DATA => put_trace_log_data(irp, &mut state),
        NUSEC_IOCTL_GET_TRACE_LOG_DATA => get_trace_log_data(irp, &state),
        NUSEC_IOCTL_GET_TRACE_LOG_STATE => write_dwords(
            irp,
            &[state.trace_head - state.trace_tail, state.trace_tail],
        ),
        NUSEC_IOCTL_GET_NVRAM_AVAILABLE => write_dwords(
            irp,
            &[TRACE_LOG_CAPACITY - (state.trace_head - state.trace_tail)],
        ),
        NUSEC_IOCTL_GET_NVRAM_GEOMETRY => write_dwords(irp, &[10, 4096]),
        _ => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
    }
}

unsafe fn read_billing_file(irp: &mut Irp, path: &str) -> i32 {
    if irp.ioctl_out.is_null() || irp.ioctl_out_nbytes == 0 {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) => {
            return iohook::hresult_from_win32(
                err.raw_os_error()
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(2),
            );
        }
    };

    // billing IOCTL 原位转换为普通读取，缓冲区较小时返回部分内容
    let output =
        std::slice::from_raw_parts_mut(irp.ioctl_out.cast::<u8>(), irp.ioctl_out_nbytes as usize);
    let read = match file.read(output) {
        Ok(read) => read,
        Err(err) => {
            return iohook::hresult_from_win32(
                err.raw_os_error()
                    .and_then(|value| u32::try_from(value).ok())
                    .unwrap_or(1),
            );
        }
    };
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = read as u32;
    }
    iohook::S_OK
}

unsafe fn put_trace_log_data(irp: &mut Irp, state: &mut NusecState) -> i32 {
    if irp.ioctl_in.is_null() || irp.ioctl_in_nbytes as usize != TRACE_LOG_RECORD_SIZE {
        return E_INVALIDARG;
    }
    if state.trace_head.wrapping_sub(state.trace_tail) >= TRACE_LOG_CAPACITY {
        return iohook::hresult_from_win32(ERROR_DISK_FULL);
    }
    let index = state.trace_head as usize % TRACE_LOG_CAPACITY as usize;
    std::ptr::copy_nonoverlapping(
        irp.ioctl_in.cast::<u8>(),
        state.trace_log[index].as_mut_ptr(),
        TRACE_LOG_RECORD_SIZE,
    );
    state.trace_head = state.trace_head.wrapping_add(1);
    iohook::S_OK
}

unsafe fn get_trace_log_data(irp: &mut Irp, state: &NusecState) -> i32 {
    if irp.ioctl_in.is_null() || irp.ioctl_in_nbytes < 8 {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    let input = std::slice::from_raw_parts(irp.ioctl_in.cast::<u8>(), 8);
    let mut position = u32::from_le_bytes(input[..4].try_into().unwrap());
    let mut count = u32::from_le_bytes(input[4..8].try_into().unwrap());
    let required = count as usize * TRACE_LOG_RECORD_SIZE;
    if irp.ioctl_out.is_null() || (irp.ioctl_out_nbytes as usize) < required {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    let mut written = 0usize;
    while count > 0 && position != state.trace_head {
        let index = position as usize % TRACE_LOG_CAPACITY as usize;
        std::ptr::copy_nonoverlapping(
            state.trace_log[index].as_ptr(),
            irp.ioctl_out.cast::<u8>().add(written),
            TRACE_LOG_RECORD_SIZE,
        );
        written += TRACE_LOG_RECORD_SIZE;
        position = position.wrapping_add(1);
        count -= 1;
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = written as u32;
    }
    iohook::S_OK
}

unsafe fn read_input_dword(irp: &Irp) -> Option<u32> {
    (irp.ioctl_in_nbytes >= 4 && !irp.ioctl_in.is_null())
        .then(|| u32::from_le_bytes(std::ptr::read_unaligned(irp.ioctl_in.cast::<[u8; 4]>())))
}

unsafe fn write_dwords(irp: &mut Irp, values: &[u32]) -> i32 {
    let size = values.len() * std::mem::size_of::<u32>();
    if irp.ioctl_out.is_null() || (irp.ioctl_out_nbytes as usize) < size {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    for (index, value) in values.iter().enumerate() {
        std::ptr::copy_nonoverlapping(
            value.to_le_bytes().as_ptr(),
            irp.ioctl_out.cast::<u8>().add(index * 4),
            4,
        );
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = size as u32;
    }
    iohook::S_OK
}

unsafe fn insufficient_input() -> i32 {
    iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER)
}

fn fixed_ascii<const N: usize>(value: &str) -> [u8; N] {
    let mut result = [0; N];
    let bytes = value.as_bytes();
    let len = bytes.len().min(N);
    result[..len].copy_from_slice(&bytes[..len]);
    result
}

unsafe fn matches_wide(value: *const u16, expected: &str) -> bool {
    applechu::platform::winapi::wide_to_string(value)
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

unsafe fn matches_ansi(value: *const u8, expected: &str) -> bool {
    (!value.is_null())
        && CStr::from_ptr(value.cast())
            .to_string_lossy()
            .eq_ignore_ascii_case(expected)
}

fn log_info(message: &str) {
    if let Some(api) = applechu::util::api::API.get() {
        api.log_info(message);
    }
}
