use std::ffi::c_char;

use once_cell::sync::OnceCell;

use crate::platform::winapi::{self, ExitWindowsExFn, RegQueryValueExAFn, RegQueryValueExWFn};
use crate::util::api::Api;

static mut ORIG_EXIT_WINDOWS_EX: Option<ExitWindowsExFn> = None;
static mut ORIG_REG_QUERY_VALUE_EX_A: Option<RegQueryValueExAFn> = None;
static mut ORIG_REG_QUERY_VALUE_EX_W: Option<RegQueryValueExWFn> = None;
static CONFIG: OnceCell<MiscConfig> = OnceCell::new();

crate::config_section! {
    pub(crate) struct MiscConfig => MISC_CONFIG_SECTION {
        section: "Misc",
        order: 950,
        default_enabled: true,
        always_enabled: false,
        hidden: true,
        comment: "其他平台行为",
        fields: {
            allow_reboot: bool = false,
            key: "allowReboot",
            comment: "允许游戏重启系统";
            allow_master_key_write: bool = false,
            key: "allowMasterKeyWrite",
            comment: "允许写入主密钥";
            next_process_file_path: String = String::new(),
            key: "nextProcessFilePath",
            comment: "下一个进程路径";
        }
    }
}

#[applechu_macros::config_section(stage = Platform, order = 50)]
pub fn init(api: &Api, config: &MiscConfig) {
    unsafe {
        let _ = CONFIG.set((*config).clone());
        ORIG_EXIT_WINDOWS_EX = winapi::hook_import(
            api,
            "user32.dll",
            "ExitWindowsEx",
            hooked_exit_windows_ex as *const (),
        );
        ORIG_REG_QUERY_VALUE_EX_A = winapi::hook_import(
            api,
            "advapi32.dll",
            "RegQueryValueExA",
            hooked_reg_query_value_ex_a as *const (),
        );
        ORIG_REG_QUERY_VALUE_EX_W = winapi::hook_import(
            api,
            "advapi32.dll",
            "RegQueryValueExW",
            hooked_reg_query_value_ex_w as *const (),
        );
    }

    api.log_info("misc platform hook initialized");
}

unsafe extern "system" fn hooked_exit_windows_ex(flags: u32, reason: u32) -> i32 {
    if CONFIG.get().is_some_and(|config| config.allow_reboot) {
        ORIG_EXIT_WINDOWS_EX.map_or(0, |orig| orig(flags, reason))
    } else {
        1
    }
}

unsafe extern "system" fn hooked_reg_query_value_ex_a(
    key: usize,
    value_name: *const c_char,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    if let Some(name) = winapi::cstr_to_string(value_name) {
        if let Some(result) = query_value(&name, data_type, data, data_len, false) {
            return result;
        }
    }

    ORIG_REG_QUERY_VALUE_EX_A.map_or(winapi::ERROR_FILE_NOT_FOUND, |orig| {
        orig(key, value_name, reserved, data_type, data, data_len)
    })
}

unsafe extern "system" fn hooked_reg_query_value_ex_w(
    key: usize,
    value_name: *const u16,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    if let Some(name) = winapi::wide_to_string(value_name) {
        if let Some(result) = query_value(&name, data_type, data, data_len, true) {
            return result;
        }
    }

    ORIG_REG_QUERY_VALUE_EX_W.map_or(winapi::ERROR_FILE_NOT_FOUND, |orig| {
        orig(key, value_name, reserved, data_type, data, data_len)
    })
}

unsafe fn query_value(
    name: &str,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
    wide: bool,
) -> Option<u32> {
    let config = CONFIG.get()?;
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "osversion" => Some(winapi::write_reg_string(
            data_type,
            data,
            data_len,
            "Microsoft Windows 10",
            wide,
        )),
        "apploadercount" | "cputemperror" | "cputempwarning" => {
            Some(winapi::write_reg_dword(data_type, data, data_len, 0))
        }
        "platformid" => Some(winapi::write_reg_string(
            data_type, data, data_len, "AAV", wide,
        )),
        "platformname" => Some(winapi::write_reg_string(
            data_type, data, data_len, "ALLS UX", wide,
        )),
        "main_nic" | "extend_nic" => Some(winapi::write_reg_string(
            data_type,
            data,
            data_len,
            "192.168.139.1",
            wide,
        )),
        "isdone" => Some(winapi::write_reg_dword(data_type, data, data_len, 1)),
        "nextprocess" => Some(winapi::write_reg_string(
            data_type,
            data,
            data_len,
            &config.next_process_file_path,
            wide,
        )),
        "allowmasterkeywrite" => Some(winapi::write_reg_dword(
            data_type,
            data,
            data_len,
            u32::from(config.allow_master_key_write),
        )),
        _ => None,
    }
}
