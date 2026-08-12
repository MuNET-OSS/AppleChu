use std::ffi::c_char;

use once_cell::sync::OnceCell;

use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use crate::platform::winapi::GetComputerNameAFn;
use crate::util::api::Api;

const ERROR_SUCCESS: u32 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

static ORIG_GET_COMPUTER_NAME_A: OnceCell<GetComputerNameAFn> = OnceCell::new();
static SERIAL_NO: OnceCell<String> = OnceCell::new();

crate::config_section! {
    pub(crate) struct PcbIdConfig => PCBID_CONFIG_SECTION {
        section: "PCBID",
        order: 350,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "PCBID 模拟",
        fields: {
            pub serial_no: String = String::from("ACAE01A99999999");
        }
    }
}

#[applechu_macros::config_section(stage = Platform, order = 80)]
pub(crate) fn init(api: &Api, config: &PcbIdConfig) -> Result<(), String> {
    if config.serial_no.len() != 15 || !config.serial_no.is_ascii() {
        return Err("PCBID serialNo must contain exactly 15 ASCII characters".to_owned());
    }
    let _ = SERIAL_NO.set(config.serial_no.clone());
    // SAFETY: detour 与 GetComputerNameA 使用相同的 system ABI 和参数布局
    unsafe {
        let mut original = std::ptr::null();
        let symbols = [HookSymbol {
            name: "GetComputerNameA",
            patch: hooked_get_computer_name_a as *const (),
            original: &mut original,
        }];
        let patched = hook_table_apply(null_module(), "kernel32.dll", &symbols);
        if !original.is_null() {
            let original = std::mem::transmute::<*const (), GetComputerNameAFn>(original);
            let _ = ORIG_GET_COMPUTER_NAME_A.set(original);
        }
        api.log_info(&format!(
            "Cabinet serial emulation ready with {patched} patched entries"
        ));
    }
    Ok(())
}

unsafe extern "system" fn hooked_get_computer_name_a(buffer: *mut c_char, size: *mut u32) -> i32 {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info("Cabinet serial requested");
    }
    if buffer.is_null() || size.is_null() {
        crate::iohook::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let Some(serial) = SERIAL_NO.get() else {
        return ORIG_GET_COMPUTER_NAME_A
            .get()
            .map_or(0, |orig| orig(buffer, size));
    };

    let required = serial.len() as u32 + 1;
    if required > *size {
        crate::iohook::set_last_error(ERROR_INSUFFICIENT_BUFFER);
        return 0;
    }

    std::ptr::copy_nonoverlapping(serial.as_ptr().cast::<c_char>(), buffer, serial.len());
    *buffer.add(serial.len()) = 0;
    *size = required - 1;
    crate::iohook::set_last_error(ERROR_SUCCESS);
    1
}
