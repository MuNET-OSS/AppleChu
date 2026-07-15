use std::ffi::{c_char, c_void};
use std::io::Write;

use windows_sys::Win32::Storage::FileSystem::WriteFile;
use windows_sys::Win32::System::Console::WriteConsoleW;

use super::state::{LoaderState, OutputSink, STATE};

extern "system" {
    fn GetLocalTime(st: *mut SYSTEMTIME);
}

#[repr(C)]
struct SYSTEMTIME {
    w_year: u16,
    w_month: u16,
    w_day_of_week: u16,
    w_day: u16,
    w_hour: u16,
    w_minute: u16,
    w_second: u16,
    w_milliseconds: u16,
}

#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    fn body_ansi(self) -> &'static str {
        match self {
            Self::Info => ANSI_GRAY,
            Self::Warn => ANSI_YELLOW,
            Self::Error => ANSI_RED,
        }
    }
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_TIME: &str = "\x1b[92m";
const ANSI_TAG: &str = "\x1b[96m";
const ANSI_GRAY: &str = "\x1b[37m";
const ANSI_YELLOW: &str = "\x1b[93m";
const ANSI_RED: &str = "\x1b[91m";

pub const ANSI_CYAN: &str = "\x1b[96m";

/// 打印一行 banner（无 [级别] 标签，正文用指定 ANSI 色；文件输出去色）
pub fn write_banner_line(state: &mut LoaderState, ansi: &str, msg: &str) {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);
        let time = format!(
            "{:02}:{:02}:{:02}.{:03}",
            st.w_hour, st.w_minute, st.w_second, st.w_milliseconds
        );

        let plain = format!("[{}] {}\n", time, msg);
        if let Some(ref mut f) = state.log_file {
            let _ = f.write_all(plain.as_bytes());
            let _ = f.flush();
        }
        if let Some(ref mut f) = state.current_mod_log_file {
            let _ = f.write_all(plain.as_bytes());
            let _ = f.flush();
        }

        let colored = format!("{ANSI_TIME}[{time}]{ANSI_RESET} {ansi}{msg}{ANSI_RESET}\n");
        write_output(state.output, &plain, &colored);
    }
}

pub fn write_log_inner(state: &mut LoaderState, msg: &str) {
    write_log_inner_level(state, LogLevel::Info, msg);
}

pub fn write_log_inner_level(state: &mut LoaderState, level: LogLevel, msg: &str) {
    unsafe {
        let mut st: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut st);

        let time = format!(
            "{:02}:{:02}:{:02}.{:03}",
            st.w_hour, st.w_minute, st.w_second, st.w_milliseconds
        );

        let plain = format!("[{}] [loader] [{}] {}\n", time, level.label(), msg);

        if let Some(ref mut f) = state.log_file {
            let _ = f.write_all(plain.as_bytes());
            let _ = f.flush();
        }
        if let Some(ref mut f) = state.current_mod_log_file {
            let _ = f.write_all(plain.as_bytes());
            let _ = f.flush();
        }

        let colored = format!(
            "{ANSI_TIME}[{time}]{ANSI_RESET} {ANSI_TAG}[loader]{ANSI_RESET} {body}[{label}] {msg}{ANSI_RESET}\n",
            body = level.body_ansi(),
            label = level.label(),
        );
        write_output(state.output, &plain, &colored);
    }
}

fn write_output(output: OutputSink, plain: &str, colored: &str) {
    match output {
        OutputSink::None => {}
        OutputSink::Console {
            handle,
            ansi_enabled,
        } => {
            let text = if ansi_enabled { colored } else { plain };
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let length = u32::try_from(utf16.len()).unwrap_or(u32::MAX);
            let mut written = 0u32;
            // SAFETY: 初始化输出时已排除无效句柄，UTF-16 缓冲区在同步调用期间保持有效。
            unsafe {
                WriteConsoleW(
                    handle,
                    utf16.as_ptr(),
                    length,
                    &mut written,
                    std::ptr::null(),
                );
            }
        }
        OutputSink::Stream(handle) => {
            let length = u32::try_from(plain.len()).unwrap_or(u32::MAX);
            let mut written = 0u32;
            // SAFETY: 初始化输出时已排除无效句柄，字符串缓冲区在同步调用期间保持有效。
            unsafe {
                WriteFile(
                    handle,
                    plain.as_ptr(),
                    length,
                    &mut written,
                    std::ptr::null_mut(),
                );
            }
        }
    }
}

pub fn log_info(msg: &str) {
    if let Ok(mut state) = STATE.lock() {
        write_log_inner_level(&mut state, LogLevel::Info, msg);
    }
}

pub fn log_warn(msg: &str) {
    if let Ok(mut state) = STATE.lock() {
        write_log_inner_level(&mut state, LogLevel::Warn, msg);
    }
}

pub fn log_error(msg: &str) {
    if let Ok(mut state) = STATE.lock() {
        write_log_inner_level(&mut state, LogLevel::Error, msg);
    }
}

#[no_mangle]
pub unsafe extern "C" fn write_log_variadic(fmt: *const c_char, args: ...) {
    extern "C" {
        fn vsnprintf(buf: *mut u8, size: usize, fmt: *const c_char, args: *const c_void) -> i32;
    }
    let mut buf = [0u8; 480];
    let len = vsnprintf(
        buf.as_mut_ptr(),
        buf.len(),
        fmt,
        &args as *const _ as *const c_void,
    );
    let len = if len < 0 {
        0
    } else {
        len.min(buf.len() as i32 - 1)
    } as usize;
    let text = String::from_utf8_lossy(&buf[..len]);
    log_info(&text);
}
