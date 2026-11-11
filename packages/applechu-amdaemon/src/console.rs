use std::ffi::{c_char, CStr};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use once_cell::sync::OnceCell;
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::{
    AllocConsole, FreeConsole, GetConsoleMode, GetStdHandle, SetConsoleMode, SetConsoleOutputCP,
    SetConsoleTitleW, WriteConsoleW, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;

use applechu::util::hash;
use applechu::util::logging::{
    format_ansi_lines, format_ansi_lines_with_body, format_lines, os_version, LogLevel, ANSI_CYAN,
    ANSI_GRAY,
};
use applechu::util::win32;

const CP_UTF8: u32 = 65001;
const TITLE: &[u16] = &[
    b'A' as u16,
    b'p' as u16,
    b'p' as u16,
    b'l' as u16,
    b'e' as u16,
    b'C' as u16,
    b'h' as u16,
    b'u' as u16,
    b' ' as u16,
    b'A' as u16,
    b'M' as u16,
    b' ' as u16,
    b'D' as u16,
    b'a' as u16,
    b'e' as u16,
    b'm' as u16,
    b'o' as u16,
    b'n' as u16,
    0,
];

static OUTPUT: AtomicUsize = AtomicUsize::new(0);
static ANSI_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_PATH: OnceCell<PathBuf> = OnceCell::new();

extern "system" {
    fn GetLocalTime(system_time: *mut SystemTime);
}

#[repr(C)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

pub(crate) fn initialize(base_dir: &str) -> bool {
    let log_path = std::path::Path::new(base_dir).join("applechu-amdaemon.log");
    let _ = std::fs::write(&log_path, []);
    let _ = std::fs::write(std::path::Path::new(base_dir).join("amdaemon.exe.log"), []);
    let _ = std::fs::remove_file(std::path::Path::new(base_dir).join("hijack_diag.log"));
    let _ = std::fs::remove_file(std::path::Path::new(base_dir).join("bootstrap_diag.log"));
    let _ = LOG_PATH.set(log_path);

    let inherited = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    let inherited = (!inherited.is_null() && inherited != INVALID_HANDLE_VALUE).then_some(inherited);

    if std::env::var_os(winhttp::amdaemon::INHERIT_CONSOLE_ENV).is_some() {
        return start_on(inherited, "AM Daemon proxy started on the game console");
    }

    // 继承来的控制台/管道属于父进程（cmd 窗口、wine 终端），不是自己开的窗口，
    // 既不需要隐藏也没法隐藏：HideWindow 只该拦住「新建控制台窗口」这一步。
    // Wine 下 AttachConsole(父进程) 不可用，父进程是 Linux shell，没有 Windows 控制台可挂
    if inherited.is_some() || winhttp::amdaemon::hide_window(base_dir) {
        return start_on(inherited, "AM Daemon proxy started");
    }

    unsafe {
        // AM Daemon 可能继承启动器的隐藏控制台，先脱离再创建可见诊断窗口
        let _ = FreeConsole();
        let allocated = AllocConsole() != 0;
        let _ = SetConsoleOutputCP(CP_UTF8);
        let _ = SetConsoleTitleW(TITLE.as_ptr());
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if !allocated || !set_output(handle, true) {
            return false;
        }
    }
    print_banner();
    crate::crash::install();
    info("AM Daemon proxy started");
    true
}

fn start_on(handle: Option<HANDLE>, message: &str) -> bool {
    // None 表示有意不往控制台写（只落文件日志），不算失败
    let ready = handle
        .map(|handle| set_output(handle, false))
        .unwrap_or(true);
    print_banner();
    crate::crash::install();
    info(message);
    ready
}

fn set_output(handle: HANDLE, configure: bool) -> bool {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return false;
    }
    OUTPUT.store(handle as usize, Ordering::Release);
    ANSI_ENABLED.store(console_ansi_enabled(handle, configure), Ordering::Release);
    true
}

fn console_ansi_enabled(handle: HANDLE, configure: bool) -> bool {
    let mut mode = 0;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return false;
    }
    if !configure {
        return mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0;
    }
    unsafe {
        SetConsoleMode(
            handle,
            mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        ) != 0
    }
}

fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    let separator = "------------------------------------------------------------";
    let hash_code = win32::module_path(print_banner as *const ())
        .and_then(hash::sha256_file)
        .unwrap_or_else(|| "unknown".to_owned());

    write_with_body(LogLevel::Info, separator, Some(ANSI_CYAN));
    write_with_body(
        LogLevel::Info,
        &format!("AppleChu v{version} Nya~ "),
        Some(ANSI_CYAN),
    );
    write_with_body(
        LogLevel::Info,
        &format!("OS: {}", os_version()),
        Some(ANSI_GRAY),
    );
    write_with_body(
        LogLevel::Info,
        &format!("Hash Code: {hash_code}"),
        Some(ANSI_GRAY),
    );
    write_with_body(LogLevel::Info, separator, Some(ANSI_CYAN));
    write_with_body(LogLevel::Info, "Service: AM Daemon", Some(ANSI_GRAY));
    write_with_body(LogLevel::Info, "Service Arch: x64", Some(ANSI_GRAY));
    write_with_body(LogLevel::Info, separator, Some(ANSI_CYAN));
}

pub(crate) fn log(message: &str) {
    info(message);
}

pub(crate) fn info(message: &str) {
    write(LogLevel::Info, message);
}

pub(crate) fn warn(message: &str) {
    write(LogLevel::Warn, message);
}

pub(crate) fn error(message: &str) {
    write(LogLevel::Error, message);
}

fn write(level: LogLevel, message: &str) {
    write_with_body(level, message, None);
}

fn write_with_body(level: LogLevel, message: &str, body: Option<&str>) {
    let mut time: SystemTime = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut time) };
    let text = format_lines(time.hour, time.minute, time.second, level, message);

    if let Some(path) = LOG_PATH.get() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = file.write_all(text.as_bytes());
            let _ = file.flush();
        }
    }

    let debug_text = text.replace('\n', "\r\n");
    let mut debug_line = debug_text.encode_utf16().collect::<Vec<_>>();
    debug_line.push(0);

    unsafe {
        OutputDebugStringW(debug_line.as_ptr());
        let handle = OUTPUT.load(Ordering::Acquire) as HANDLE;
        if handle.is_null() {
            return;
        }
        let colored;
        let console_text = if ANSI_ENABLED.load(Ordering::Acquire) {
            colored = match body {
                Some(body) => format_ansi_lines_with_body(
                    time.hour,
                    time.minute,
                    time.second,
                    level,
                    message,
                    body,
                ),
                None => format_ansi_lines(time.hour, time.minute, time.second, level, message),
            };
            colored.as_str()
        } else {
            text.as_str()
        };
        let console_text = console_text.replace('\n', "\r\n");
        let line = console_text.encode_utf16().collect::<Vec<_>>();
        let mut written = 0;
        if WriteConsoleW(
            handle,
            line.as_ptr(),
            line.len() as u32,
            &mut written,
            std::ptr::null(),
        ) == 0
        {
            // 句柄被重定向到文件或管道时不是控制台，WriteConsoleW 必然失败，按字节写回退
            let plain = text.replace('\n', "\r\n");
            let _ = WriteFile(
                handle,
                plain.as_ptr(),
                plain.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            );
        }
    }
}

/// 供 AppleChu 独立 Api 使用的日志 ABI
pub(crate) unsafe extern "C" fn standalone_logger(level: LogLevel, message: *const c_char) {
    if message.is_null() {
        return;
    }
    let message = CStr::from_ptr(message).to_string_lossy();
    write(level, &message);
}
