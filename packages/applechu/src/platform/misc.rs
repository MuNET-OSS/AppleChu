use once_cell::sync::OnceCell;

use crate::platform::reg_hook::{self, RegValue, HKEY_LOCAL_MACHINE};
use crate::platform::winapi::{self, ExitWindowsExFn};
use crate::util::api::Api;

static ORIG_EXIT_WINDOWS_EX: OnceCell<ExitWindowsExFn> = OnceCell::new();
static CONFIG: OnceCell<MiscConfig> = OnceCell::new();

crate::config_section! {
    pub(crate) struct MiscConfig => MISC_CONFIG_SECTION {
        section: "Misc",
        order: 950,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "其他平台行为",
        fields: {
            allow_reboot: bool = false,
            comment: "允许游戏重启系统";
            allow_master_key_write: bool = false,
            comment: "允许写入主密钥";
            next_process_file_path: String = String::new(),
            comment: "下一个进程路径";
        }
    }
}

#[applechu_macros::config_section(stage = Platform, order = 50)]
pub(crate) fn init(api: &Api, config: &MiscConfig) {
    register_platform_keys(config);
    let _ = CONFIG.set((*config).clone());
    // SAFETY: detour 与 ExitWindowsEx 使用相同的 system ABI 和参数布局
    unsafe {
        if let Some(original) = winapi::hook_import(
            api,
            "user32.dll",
            "ExitWindowsEx",
            hooked_exit_windows_ex as *const (),
        ) {
            let _ = ORIG_EXIT_WINDOWS_EX.set(original);
        }
    }

    api.log_info("Platform control protection ready");
}

fn register_platform_keys(config: &MiscConfig) {
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty",
        vec![RegValue::string("OSVersion", "0_0_0")],
    );
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty\\static",
        vec![
            RegValue::dword("CpuTempError", 100),
            RegValue::dword("CpuTempWarning", 95),
            RegValue::string("PlatformId", "ACA1"),
            RegValue::string("PlatformName", "ALLS MX2.1"),
        ],
    );
    if !config.allow_master_key_write {
        reg_hook::push_key(
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\SEGA\\SystemProperty\\Master",
            vec![
                RegValue::dword("AppLoaderCount", 1),
                RegValue::string("NextProcess", &config.next_process_file_path),
                RegValue::string("SystemError", ""),
            ],
        );
    }
}

unsafe extern "system" fn hooked_exit_windows_ex(flags: u32, reason: u32) -> i32 {
    if CONFIG.get().is_some_and(|config| config.allow_reboot) {
        ORIG_EXIT_WINDOWS_EX
            .get()
            .map_or(0, |orig| orig(flags, reason))
    } else {
        1
    }
}
