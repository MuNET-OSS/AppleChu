mod stack_trace;

use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Write;
use std::mem::zeroed;
use std::ptr::null;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS;

use self::stack_trace::{append_registers, append_stack_trace, module_offset, NativeContext};
use super::log::{log_error, log_info};
use super::pe::get_self_base_dir;
use super::{crash_ui, crash_zip};

const EXCEPTION_EXECUTE_HANDLER: i32 = 1;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;
const EXCEPTION_STACK_OVERFLOW: u32 = 0xC00000FD;

static CRASH_HANDLING: AtomicBool = AtomicBool::new(false);

extern "system" {
    fn CreateDirectoryA(path: *const u8, security: *const c_void) -> i32;
    fn GetLocalTime(st: *mut SYSTEMTIME);
    fn SetUnhandledExceptionFilter(
        filter: Option<unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32>,
    ) -> Option<unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32>;
    fn AddVectoredExceptionHandler(
        first: u32,
        handler: unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32,
    ) -> *mut c_void;
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

pub unsafe fn install() {
    // VEH（first=1，最高优先级）：异常分发最早期捕获，不会被游戏/第三方 DLL 后装的
    // UEF 顶掉，覆盖游戏本体与其他环境的致命崩溃
    AddVectoredExceptionHandler(1, vectored_handler);
    // UEF 作为第二道防线，兜底 VEH 放行的未处理异常
    SetUnhandledExceptionFilter(Some(unhandled_exception_filter));
    log_info("crash handler installed (VEH + UEF)");
}

/// 仅对明确致命的异常码弹崩溃窗口；放行 C++ 异常 / 断点 / 普通 first-chance，
/// 避免误杀游戏正常的 __try/__except 流程
unsafe extern "system" fn vectored_handler(exception: *mut EXCEPTION_POINTERS) -> i32 {
    if exception.is_null() || (*exception).ExceptionRecord.is_null() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let record = &*(*exception).ExceptionRecord;
    let code = record.ExceptionCode as u32;

    // STACK_OVERFLOW 必须由 VEH 处理：UEF 在栈耗尽时无法可靠运行。
    // 其余致命码放行给 UEF（last-chance 语义，确保确实无人处理才弹），
    // 这样不会误杀游戏 first-chance 后自行恢复的 SEH。
    if code != EXCEPTION_STACK_OVERFLOW {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    if CRASH_HANDLING.swap(true, Ordering::SeqCst) {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    handle_crash(exception);
    EXCEPTION_EXECUTE_HANDLER
}

unsafe extern "system" fn unhandled_exception_filter(exception: *mut EXCEPTION_POINTERS) -> i32 {
    if CRASH_HANDLING.swap(true, Ordering::SeqCst) {
        return EXCEPTION_EXECUTE_HANDLER;
    }
    handle_crash(exception);
    EXCEPTION_EXECUTE_HANDLER
}

unsafe fn handle_crash(exception: *mut EXCEPTION_POINTERS) {
    let report = build_crash_text(exception);
    log_error("game crashed Nya... (>_<)");
    for line in report.lines() {
        log_error(line);
    }

    let base_dir = get_self_base_dir().unwrap_or_default();
    let crash_dir = format!("{}\\mods\\crash", base_dir);
    CreateDirectoryA(format!("{}\\mods\0", base_dir).as_ptr(), null());
    CreateDirectoryA(format!("{}\0", crash_dir).as_ptr(), null());

    let stamp = timestamp();
    let log_path = format!("{}\\crash_{}.log", crash_dir, stamp);
    if let Ok(mut file) = File::create(&log_path) {
        let _ = file.write_all(report.as_bytes());
    }

    let zip_path = crash_zip::build(&base_dir, &crash_dir, &stamp, &log_path);

    crash_ui::show(&report, &crash_dir, zip_path.as_deref());
}

unsafe fn build_crash_text(exception: *mut EXCEPTION_POINTERS) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "=== Chusan Crash Report Nya... (>_<) ===");
    let _ = writeln!(out);

    if exception.is_null() || (*exception).ExceptionRecord.is_null() {
        let _ = writeln!(out, "exception: <null> (unknown reason Nya...)");
        return out;
    }

    let record = &*(*exception).ExceptionRecord;
    let code = record.ExceptionCode as u32;
    let _ = writeln!(
        out,
        "exception_code: 0x{:08X} ({})",
        code,
        exception_name(code)
    );
    let _ = writeln!(
        out,
        "exception_address: 0x{:08X}",
        record.ExceptionAddress as usize
    );
    if let Some(module) = module_offset(record.ExceptionAddress as usize) {
        let _ = writeln!(out, "exception_module: {}+0x{:X}", module.0, module.1);
    }

    #[cfg(target_arch = "x86")]
    if !(*exception).ContextRecord.is_null() {
        append_registers(
            &mut out,
            &*((*exception).ContextRecord as *const NativeContext),
        );
        append_stack_trace(
            &mut out,
            &mut *((*exception).ContextRecord as *mut NativeContext),
        );
    }

    out
}

fn exception_name(code: u32) -> &'static str {
    match code {
        0xC0000005 => "ACCESS_VIOLATION",
        0xC000001D => "ILLEGAL_INSTRUCTION",
        0xC0000094 => "INT_DIVIDE_BY_ZERO",
        0xC00000FD => "STACK_OVERFLOW",
        0x80000003 => "BREAKPOINT",
        0xC0000025 => "NONCONTINUABLE_EXCEPTION",
        _ => "unknown",
    }
}

unsafe fn timestamp() -> String {
    let mut st: SYSTEMTIME = zeroed();
    GetLocalTime(&mut st);
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}",
        st.w_year, st.w_month, st.w_day, st.w_hour, st.w_minute, st.w_second, st.w_milliseconds
    )
}

pub fn log_panic_context(scope: &str, name: &str) {
    let base_dir = get_self_base_dir().unwrap_or_default();
    if base_dir.is_empty() {
        log_error(&format!("panic caught in {}: {}", scope, name));
        return;
    }
    let crash_dir = format!("{}\\mods\\crash", base_dir);
    let _ = fs::create_dir_all(&crash_dir);
    let stamp = unsafe { timestamp() };
    let path = format!("{}\\panic_{}.log", crash_dir, stamp);
    let report = format!(
        "=== Chusan Panic Nya... (>_<) ===\n\nscope: {}\nmod:   {}\n",
        scope, name
    );
    if let Ok(mut file) = File::create(&path) {
        let _ = file.write_all(report.as_bytes());
    }
    log_error(&format!(
        "panic caught in {}: {} (context={})",
        scope, name, path
    ));
    crash_ui::show(&report, &crash_dir, None);
}
