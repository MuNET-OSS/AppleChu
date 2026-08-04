use std::ffi::CStr;

use windows_sys::Win32::Foundation::HANDLE;

use applechu::config::Config;
use applechu::iohook::{self, Irp, IrpOp};
use applechu::util::api::Api;

const HWMON_IOCTL_READ_CPU_TEMP: u32 = 0x8000_6000;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
static mut HWMON_FD: HANDLE = std::ptr::null_mut();

#[applechu_macros::config_section(stage = Platform, order = 40)]
pub fn init(api: &Api, _config: &Config) {
    unsafe {
        let Some(fd) = iohook::open_nul_fd() else {
            return api.log_warn("Hwmon emulator failed to create a device handle");
        };
        HWMON_FD = fd;
        if !iohook::push_handler(handle_irp) {
            return api.log_warn("Hwmon emulator failed to register its device handler");
        }
    }
    api.log_info("Hwmon emulator ready");
}

unsafe fn handle_irp(irp: &mut Irp) -> i32 {
    if irp.op != IrpOp::Open && irp.fd != HWMON_FD {
        return iohook::invoke_next(irp);
    }
    match irp.op {
        IrpOp::Open
            if matches_wide(irp.open_filename_w, "\\\\.\\sghwmonitor")
                || matches_ansi(irp.open_filename_a, "\\\\.\\sghwmonitor") =>
        {
            irp.fd = HWMON_FD;
            log_info("Hwmon device opened");
            iohook::S_OK
        }
        IrpOp::Open => iohook::invoke_next(irp),
        IrpOp::Close => {
            log_info("Hwmon device closed");
            iohook::S_OK
        }
        IrpOp::Ioctl if irp.ioctl == HWMON_IOCTL_READ_CPU_TEMP => write_dword(irp, 52),
        IrpOp::Ioctl => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
        _ => iohook::hresult_from_win32(ERROR_INVALID_FUNCTION),
    }
}

unsafe fn write_dword(irp: &mut Irp, value: u32) -> i32 {
    if irp.ioctl_out.is_null() || irp.ioctl_out_nbytes < 4 {
        return iohook::hresult_from_win32(ERROR_INSUFFICIENT_BUFFER);
    }
    std::ptr::copy_nonoverlapping(value.to_le_bytes().as_ptr(), irp.ioctl_out.cast(), 4);
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 4;
    }
    iohook::S_OK
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
