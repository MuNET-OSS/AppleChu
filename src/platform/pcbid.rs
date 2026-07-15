use std::ffi::c_char;

use once_cell::sync::OnceCell;

use crate::config::Config;
use crate::platform::winapi::{self, GetComputerNameAFn};
use crate::util::api::Api;

static mut ORIG_GET_COMPUTER_NAME_A: Option<GetComputerNameAFn> = None;
static SERIAL_NO: OnceCell<String> = OnceCell::new();

crate::config_section! {
    pub(crate) struct PcbIdConfig => PCBID_CONFIG_SECTION {
        section: "PCBID",
        order: 960,
        default_enabled: true,
        always_enabled: false,
        hidden: true,
        comment: "机台序列号模拟",
        fields: {
            pub serial_no: String = String::from("A69E01A8888"),
            key: "serialNo",
            comment: "机台序列号";
        }
    }
}

pub fn init(api: &Api, config: &Config) {
    let Some(config) = config
        .section::<PcbIdConfig>()
        .filter(|config| config.enabled)
    else {
        return;
    };

    unsafe {
        let _ = SERIAL_NO.set(config.serial_no.clone());
        ORIG_GET_COMPUTER_NAME_A = winapi::hook_import(
            api,
            "kernel32.dll",
            "GetComputerNameA",
            hooked_get_computer_name_a as *const (),
        );
    }

    api.log_info("PCBID hook initialized");
}

pub fn shutdown() {}

unsafe extern "system" fn hooked_get_computer_name_a(buffer: *mut c_char, size: *mut u32) -> i32 {
    let Some(serial) = SERIAL_NO.get() else {
        return ORIG_GET_COMPUTER_NAME_A.map_or(0, |orig| orig(buffer, size));
    };

    let required = serial.len() as u32;
    if size.is_null() {
        return 0;
    }
    if buffer.is_null() || *size <= required {
        *size = required + 1;
        return 0;
    }

    std::ptr::copy_nonoverlapping(serial.as_ptr().cast::<c_char>(), buffer, serial.len());
    *buffer.add(serial.len()) = 0;
    *size = required;
    1
}
