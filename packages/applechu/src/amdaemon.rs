//! 供游戏侧启动和 AM Daemon 侧共享的配置支持

use crate::config::DiagnosticLevel;
use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use crate::util::api::{Api, StandaloneLogger, API};

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use std::ffi::OsStr;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::ptr::{null, null_mut};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject,
    CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

const CONFIG_FILE_PATTERN: &str = "config_*.json";
const STANDARD_CONFIG_ORDER: [&str; 6] = [
    "config_common.json",
    "config_server.json",
    "config_client.json",
    "config_cvt.json",
    "config_sp.json",
    "config_hook.json",
];

pub const INHERIT_CONSOLE_ENV: &str = "APPLECHU_AMDAEMON_INHERIT_CONSOLE";

#[cfg(windows)]
static AUTO_STARTED_CHILD: OnceLock<Mutex<Option<AutoStartedChild>>> = OnceLock::new();
#[cfg(windows)]
static TERMINATE_AUTO_STARTED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
struct AutoStartedChild {
    process: usize,
    job: usize,
}

#[cfg(windows)]
impl AutoStartedChild {
    fn process_handle(&self) -> HANDLE {
        self.process as HANDLE
    }

    fn job_handle(&self) -> HANDLE {
        self.job as HANDLE
    }

    fn try_wait(&mut self) -> Option<u32> {
        let status = unsafe { WaitForSingleObject(self.process_handle(), 0) };
        if status == WAIT_TIMEOUT {
            return None;
        }
        let mut exit_code = 1;
        if status == WAIT_OBJECT_0 {
            let _ = unsafe { GetExitCodeProcess(self.process_handle(), &mut exit_code) };
        }
        Some(exit_code)
    }

    fn stop(&mut self) {
        unsafe {
            if self.job != 0 {
                let _ = TerminateJobObject(self.job_handle(), 1);
                let _ = WaitForSingleObject(self.process_handle(), 5_000);
            } else {
                let _ = TerminateProcess(self.process_handle(), 1);
                let _ = WaitForSingleObject(self.process_handle(), 5_000);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for AutoStartedChild {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.process_handle());
            if self.job != 0 {
                let _ = CloseHandle(self.job_handle());
            }
        }
    }
}

crate::config_section! {
    pub(crate) struct AmdaemonConfig => AMDAEMON_CONFIG_SECTION {
        section: "Amdaemon",
        order: 5,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "AM Daemon x64 winmm 劫持配置",
        fields: {
            pub auto_start: bool = false,
            key: "AutoStart",
            emit_default: true,
            comment: "由游戏侧启动 AM Daemon";
            pub executable: String = String::from("amdaemon.exe"),
            key: "Executable",
            comment: "AM Daemon 可执行文件名";
            pub hide_window: bool = false,
            key: "HideWindow",
            comment: "手动启动 AM Daemon 时隐藏控制台窗口";
            pub terminate_on_exit: bool = true,
            key: "TerminateOnExit",
            comment: "AppleChu 退出时终止 AM Daemon";
            pub append_config_args: bool = false,
            key: "AppendConfigArgs",
            emit_default: true,
            comment: "无完整 -c 参数时补充 JSON 配置";
            pub config_files: Vec<String> = vec![CONFIG_FILE_PATTERN.to_owned()],
            key: "ConfigFiles",
            comment: "AM Daemon JSON 配置文件列表";
        }
    }
}

crate::config_section! {
    pub struct DnsConfig => DNS_CONFIG_SECTION {
        section: "Dns",
        order: 980,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "AM 平台 DNS 映射",
        fields: {
            pub default: String = String::new(),
            comment: "未单独指定时使用的服务器地址";
            pub router: String = String::new(),
            comment: "店内路由服务器";
            pub startup: String = String::new(),
            comment: "启动认证服务器";
            pub billing: String = String::new(),
            comment: "计费服务器";
            pub aimedb: String = String::new(),
            comment: "AimeDB 服务器";
            pub title: String = String::new(),
            comment: "标题/其他服务器";
            pub replace_host: bool = false,
            key: "replaceHost",
            comment: "替换 HTTP Host";
            pub startup_port: u16 = 0,
            key: "startupPort",
            comment: "启动认证服务器端口";
            pub billing_port: u16 = 0,
            key: "billingPort",
            comment: "计费服务器端口";
            pub aimedb_port: u16 = 0,
            key: "aimedbPort",
            comment: "AimeDB 服务器端口";
        }
    }
}

crate::config_section! {
    pub struct KeychipConfig => KEYCHIP_CONFIG_SECTION {
        section: "Keychip",
        order: 970,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "NuSec/keychip 模拟",
        fields: {
            pub keychip_id: String = String::from("A69E-01A88888888"),
            key: "id",
            comment: "Keychip ID";
            pub game_id: String = String::from("SDHD"),
            key: "gameId",
            comment: "游戏 ID，默认 SDHD";
            pub platform_id: String = String::new(),
            key: "platformId",
            comment: "平台 ID；留空时使用当前平台默认值";
            pub region: u32 = 1,
            comment: "区域编号";
            pub billing_type: u32 = 1,
            key: "billingType",
            comment: "计费类型";
            pub system_flag: u32 = 0x64,
            key: "systemFlag",
            comment: "系统标志";
            pub subnet: String = String::from("192.168.139.0"),
            comment: "店内网络子网";
            pub billing_ca: String = String::from("DEVICE\\ca.crt"),
            key: "billingCa",
            comment: "计费 CA 证书";
            pub billing_pub: String = String::from("DEVICE\\billing.pub"),
            key: "billingPub",
            comment: "计费公钥";
        }
    }
}

crate::config_section! {
    pub struct NetEnvConfig => NETENV_CONFIG_SECTION {
        section: "NetEnv",
        order: 975,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "店内网络适配器模拟",
        fields: {
            pub addr_suffix: u32 = 11,
            key: "addrSuffix",
            comment: "机台 IP 的末尾地址";
            pub router_suffix: u32 = 254,
            key: "routerSuffix",
            comment: "店内路由 IP 的末尾地址";
            pub mac_addr: String = String::from("01:02:03:04:05:06"),
            key: "macAddr",
            comment: "虚拟网卡 MAC 地址";
            pub broadcast: String = String::from("255.255.255.255"),
            comment: "UDP 广播目标地址";
        }
    }
}

crate::config_section! {
    pub struct EpayConfig => EPAY_CONFIG_SECTION {
        section: "Epay",
        order: 972,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "AM Daemon ThincaPayment 兼容",
        fields: {
            pub hook: bool = true,
            comment: "使用本地支付接口桩，使 AM Daemon 可在无支付终端时启动";
        }
    }
}

crate::config_section! {
    pub struct OpenSslConfig => OPENSSL_CONFIG_SECTION {
        section: "OpenSsl",
        order: 975,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "AM Daemon OpenSSL 兼容",
        fields: {
            pub force_legacy_sha: bool = false,
            key: "forceLegacySha",
            comment: "强制禁用 OpenSSL SHA 扩展路径";
        }
    }
}

pub fn config_files(base_dir: &str) -> Vec<String> {
    let config = crate::config::Config::global(base_dir);
    let configured = config
        .section::<AmdaemonConfig>()
        .filter(|section| section.enabled)
        .map(|section| section.config_files.clone())
        .unwrap_or_default();
    resolve_config_files(std::path::Path::new(base_dir), &configured)
}

fn resolve_config_files(base_dir: &std::path::Path, configured: &[String]) -> Vec<String> {
    if configured.is_empty() {
        return discover_config_files(base_dir);
    }

    let mut resolved = Vec::new();
    for file in configured {
        if file.eq_ignore_ascii_case(CONFIG_FILE_PATTERN) {
            for discovered in discover_config_files(base_dir) {
                if !resolved
                    .iter()
                    .any(|current: &String| current.eq_ignore_ascii_case(&discovered))
                {
                    resolved.push(discovered);
                }
            }
        } else if !resolved
            .iter()
            .any(|current| current.eq_ignore_ascii_case(file))
        {
            resolved.push(file.clone());
        }
    }
    resolved
}

fn discover_config_files(base_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(base_dir) else {
        return Vec::new();
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|file_type| file_type.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            let name = name.to_ascii_lowercase();
            name.starts_with("config_") && name.ends_with(".json")
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        config_file_rank(left)
            .cmp(&config_file_rank(right))
            .then_with(|| left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()))
            .then_with(|| left.cmp(right))
    });
    files
}

fn config_file_rank(name: &str) -> usize {
    STANDARD_CONFIG_ORDER
        .iter()
        .position(|standard| name.eq_ignore_ascii_case(standard))
        .unwrap_or(STANDARD_CONFIG_ORDER.len())
}

pub fn append_config_args(base_dir: &str) -> bool {
    crate::config::Config::global(base_dir)
        .section::<AmdaemonConfig>()
        .is_some_and(|section| section.enabled && section.append_config_args)
}

pub fn hide_window(base_dir: &str) -> bool {
    crate::config::Config::global(base_dir)
        .section::<AmdaemonConfig>()
        .is_some_and(|section| section.enabled && section.hide_window)
}

/// 在游戏侧 winhttp 加载后异步启动 AM Daemon，避免在 DllMain 中调用进程创建 API
#[cfg(windows)]
pub fn auto_start(base_dir: &str) {
    let config = crate::config::Config::global(base_dir);
    let Some(section) = config.section::<AmdaemonConfig>() else {
        return;
    };
    if !section.enabled || !section.auto_start {
        return;
    }

    let base_dir = std::path::Path::new(base_dir).to_owned();
    let executable = section.executable.clone();
    let terminate_on_exit = section.terminate_on_exit;
    let config_files = config_files(base_dir.to_string_lossy().as_ref());
    if config_files.is_empty() {
        log_error("No AM Daemon config_*.json files were found");
        return;
    }
    TERMINATE_AUTO_STARTED.store(terminate_on_exit, Ordering::Release);

    std::thread::spawn(move || {
        let children = AUTO_STARTED_CHILD.get_or_init(|| Mutex::new(None));
        let Ok(mut guard) = children.lock() else {
            log_error("Unable to access AM Daemon child process state");
            return;
        };

        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                None => return,
                Some(_) => *guard = None,
            }
        }

        let executable_path = std::path::Path::new(&executable);
        let executable_path = if executable_path.is_absolute() {
            executable_path.to_owned()
        } else {
            base_dir.join(executable_path)
        };
        match spawn_auto_started(
            &executable_path,
            &base_dir,
            &config_files,
            terminate_on_exit,
        ) {
            Ok(child) => {
                log_info(&format!(
                    "AM Daemon started with output attached to the game console: {}{}",
                    executable_path.display(),
                    if terminate_on_exit {
                        " (job managed)"
                    } else {
                        ""
                    }
                ));
                *guard = Some(child);
            }
            Err(error) => log_error(&format!(
                "Failed to start AM Daemon: {}: {error}",
                executable_path.display()
            )),
        }
    });
}

#[cfg(windows)]
fn spawn_auto_started(
    executable: &std::path::Path,
    base_dir: &std::path::Path,
    config_files: &[String],
    terminate_on_exit: bool,
) -> Result<AutoStartedChild, String> {
    let mut application = wide_path(executable);
    let mut command_line = wide_command_line(executable, config_files);
    let mut environment = wide_environment();
    let current_directory = wide_path(base_dir);
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let creation_flags = CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;

    let created = unsafe {
        CreateProcessW(
            application.as_mut_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            creation_flags,
            environment.as_mut_ptr().cast(),
            current_directory.as_ptr(),
            &startup,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(format!("CreateProcessW failed ({})", unsafe {
            GetLastError()
        }));
    }

    let job = if terminate_on_exit {
        let job = unsafe { CreateJobObjectW(null(), null()) };
        if job.is_null() {
            let error = unsafe { GetLastError() };
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
            }
            return Err(format!("CreateJobObjectW failed ({error})"));
        }

        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } != 0;
        let assigned =
            configured && unsafe { AssignProcessToJobObject(job, process_info.hProcess) } != 0;
        if !assigned {
            let error = unsafe { GetLastError() };
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                let _ = CloseHandle(process_info.hThread);
                let _ = CloseHandle(process_info.hProcess);
                let _ = CloseHandle(job);
            }
            return Err(format!("AM Daemon job setup failed ({error})"));
        }
        job
    } else {
        null_mut()
    };

    if unsafe { ResumeThread(process_info.hThread) } == u32::MAX {
        let error = unsafe { GetLastError() };
        unsafe {
            if !job.is_null() {
                let _ = TerminateJobObject(job, 1);
            } else {
                let _ = TerminateProcess(process_info.hProcess, 1);
            }
            let _ = CloseHandle(process_info.hThread);
            let _ = CloseHandle(process_info.hProcess);
            if !job.is_null() {
                let _ = CloseHandle(job);
            }
        }
        return Err(format!("ResumeThread failed ({error})"));
    }

    unsafe {
        let _ = CloseHandle(process_info.hThread);
    }

    Ok(AutoStartedChild {
        process: process_info.hProcess as usize,
        job: job as usize,
    })
}

#[cfg(windows)]
fn wide_path(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn wide_command_line(executable: &std::path::Path, config_files: &[String]) -> Vec<u16> {
    let mut arguments = vec![
        quote_windows_arg(&executable.to_string_lossy()),
        "-c".to_owned(),
    ];
    arguments.extend(config_files.iter().map(|file| quote_windows_arg(file)));
    OsStr::new(&arguments.join(" "))
        .encode_wide()
        .chain(Some(0))
        .collect()
}

#[cfg(windows)]
fn quote_windows_arg(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        match character {
            '\\' => slashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                quoted.push('"');
                slashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', slashes));
                quoted.push(character);
                slashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(windows)]
fn wide_environment() -> Vec<u16> {
    let mut entries = std::env::vars_os()
        .filter(|(key, _)| {
            !key.to_string_lossy()
                .eq_ignore_ascii_case(INHERIT_CONSOLE_ENV)
        })
        .map(|(key, value)| {
            let mut entry = key;
            entry.push("=");
            entry.push(value);
            entry
        })
        .collect::<Vec<_>>();
    let mut inherit_console = OsStr::new(INHERIT_CONSOLE_ENV).to_os_string();
    inherit_console.push("=1");
    entries.push(inherit_console);
    let mut environment = Vec::new();
    for entry in entries {
        environment.extend(entry.encode_wide());
        environment.push(0);
    }
    environment.push(0);
    environment
}

#[cfg(not(windows))]
pub fn auto_start(_base_dir: &str) {}

#[cfg(windows)]
pub fn stop_auto_started() {
    if !TERMINATE_AUTO_STARTED.load(Ordering::Acquire) {
        return;
    }
    let Some(children) = AUTO_STARTED_CHILD.get() else {
        return;
    };
    let Ok(mut guard) = children.lock() else {
        return;
    };
    if let Some(mut child) = guard.take() {
        child.stop();
        log_info("Stopped the automatically started AM Daemon");
    }
}

#[cfg(not(windows))]
pub fn stop_auto_started() {}

fn log_info(message: &str) {
    if let Some(api) = API.get() {
        api.log_info(message);
    }
}

fn log_error(message: &str) {
    if let Some(api) = API.get() {
        api.log_error(message);
    }
}

/// 在 AM Daemon 的 CRT 初始化前替换 ANSI 和宽字符命令行入口
pub unsafe fn install_command_line_hooks(
    get_command_line_a: *const (),
    get_command_line_w: *const (),
) -> usize {
    let symbols = [
        HookSymbol {
            name: "GetCommandLineA",
            patch: get_command_line_a,
            original: std::ptr::null_mut(),
        },
        HookSymbol {
            name: "GetCommandLineW",
            patch: get_command_line_w,
            original: std::ptr::null_mut(),
        },
    ];
    hook_table_apply(null_module(), "kernel32.dll", &symbols)
}

/// 安装 AM Daemon 自身导入的 `__wgetmainargs` hook，并返回原函数地址
pub unsafe fn install_wgetmainargs_hook(replacement: *const (), original: *mut *const ()) -> usize {
    let symbols = [HookSymbol {
        name: "__wgetmainargs",
        patch: replacement,
        original,
    }];
    hook_table_apply(null_module(), "msvcr110.dll", &symbols)
}

/// 初始化公共 AM Daemon 运行时，并按调用方提供的顺序启动其专用模块
pub fn initialize(
    base_dir: &str,
    logger: StandaloneLogger,
    module_order: &[&str],
) -> Result<(), String> {
    let api =
        Api::standalone(logger).ok_or_else(|| "failed to inspect AM Daemon PE image".to_owned())?;
    api.install();
    let api = API
        .get()
        .ok_or_else(|| "failed to install standalone API".to_owned())?;
    let config = crate::config::Config::global(base_dir);

    for diagnostic in config.diagnostics() {
        match diagnostic.level {
            DiagnosticLevel::Warning => api.log_warn(&diagnostic.message),
            DiagnosticLevel::Error => api.log_error(&diagnostic.message),
        }
    }
    if !config.is_valid() {
        return Err("AppleChu.toml is invalid".to_owned());
    }

    crate::module_registry::init_ordered(api, config, module_order);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{discover_config_files, resolve_config_files, CONFIG_FILE_PATTERN};

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "applechu-amdaemon-config-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("系统时间必须有效")
                    .as_nanos()
            ));
            std::fs::create_dir(&path).expect("测试目录必须可创建");
            Self(path)
        }

        fn create(&self, name: &str) {
            std::fs::write(self.0.join(name), []).expect("测试文件必须可创建");
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovers_all_json_configs_in_standard_order() {
        let directory = TestDirectory::new();
        for name in [
            "config_extra.json",
            "config_sp.json",
            "config_common.json",
            "config_server.json.bak",
            "other.json",
        ] {
            directory.create(name);
        }

        assert_eq!(
            discover_config_files(&directory.0),
            ["config_common.json", "config_sp.json", "config_extra.json"]
        );
        assert_eq!(
            resolve_config_files(&directory.0, &[CONFIG_FILE_PATTERN.to_owned()]),
            ["config_common.json", "config_sp.json", "config_extra.json"]
        );
    }

    #[test]
    fn explicit_config_files_keep_their_order() {
        let configured = vec!["second.json".to_owned(), "first.json".to_owned()];
        assert_eq!(
            resolve_config_files(std::path::Path::new("."), &configured),
            configured
        );
    }
}
