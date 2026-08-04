use std::ffi::c_void;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use applechu::iohook::hook_table::{hook_table_apply, HookSymbol};
use applechu::util::win32;
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddrModeFlat, RtlCaptureStackBackTrace, SetUnhandledExceptionFilter, StackWalk64, SymCleanup,
    SymFunctionTableAccess64, SymGetModuleBase64, SymInitialize, CONTEXT, EXCEPTION_POINTERS,
    LPTOP_LEVEL_EXCEPTION_FILTER, STACKFRAME64,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};

const EXCEPTION_EXECUTE_HANDLER: i32 = 1;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REPORTING: AtomicBool = AtomicBool::new(false);

pub(crate) fn install() {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }

    unsafe {
        SetUnhandledExceptionFilter(Some(unhandled_exception));
        let executable = GetModuleHandleW(std::ptr::null());
        let mut patched = 0;
        if !executable.is_null() {
            let symbols = [HookSymbol {
                name: "SetUnhandledExceptionFilter",
                patch: hooked_set_unhandled_exception_filter as *const (),
                original: std::ptr::null_mut(),
            }];
            patched = hook_table_apply(executable, "kernel32.dll", &symbols);
        }
        crate::console::info(&format!(
            "AM Daemon crash handler ready with {patched} protected entries"
        ));
    }
}

unsafe extern "system" fn hooked_set_unhandled_exception_filter(
    _filter: LPTOP_LEVEL_EXCEPTION_FILTER,
) -> LPTOP_LEVEL_EXCEPTION_FILTER {
    SetUnhandledExceptionFilter(Some(unhandled_exception))
}

unsafe extern "system" fn unhandled_exception(info: *const EXCEPTION_POINTERS) -> i32 {
    if REPORTING.swap(true, Ordering::AcqRel) {
        return EXCEPTION_EXECUTE_HANDLER;
    }

    let report = build_exception_report(info);
    emit_report("amdaemon_crash", "AM Daemon crashed", &report);
    EXCEPTION_EXECUTE_HANDLER
}

pub(crate) fn report_exit(source: &str, code: i32) {
    if REPORTING.swap(true, Ordering::AcqRel) {
        return;
    }

    let mut report = String::new();
    let _ = writeln!(report, "=== AppleChu AM Daemon Exit Report ===\n");
    let _ = writeln!(report, "source: {source}");
    let _ = writeln!(report, "exit_code: {code:#010x}\n");
    unsafe { append_current_stack(&mut report, 2) };
    emit_report("amdaemon_exit", "AM Daemon exited unexpectedly", &report);
}

pub(crate) fn report_startup_failure(message: &str) {
    if REPORTING.swap(true, Ordering::AcqRel) {
        return;
    }

    let mut report = String::new();
    let _ = writeln!(report, "=== AppleChu AM Daemon Startup Failure ===\n");
    let _ = writeln!(report, "reason: {message}\n");
    unsafe { append_current_stack(&mut report, 2) };
    emit_report("amdaemon_startup", "AM Daemon startup failed", &report);
}

unsafe fn build_exception_report(info: *const EXCEPTION_POINTERS) -> String {
    let mut report = String::new();
    let _ = writeln!(report, "=== AppleChu AM Daemon Crash Report ===\n");
    if info.is_null() || (*info).ExceptionRecord.is_null() {
        let _ = writeln!(report, "exception: unavailable");
        return report;
    }

    let record = &*(*info).ExceptionRecord;
    let code = record.ExceptionCode as u32;
    let address = record.ExceptionAddress as usize;
    let _ = writeln!(
        report,
        "exception_code: {code:#010x} ({})",
        exception_name(code)
    );
    let _ = writeln!(report, "exception_address: {address:#018x}");
    if let Some(location) = module_location(address) {
        let _ = writeln!(report, "exception_module: {location}");
    }

    if !(*info).ContextRecord.is_null() {
        let mut context = *(*info).ContextRecord;
        append_registers(&mut report, &context);
        append_context_stack(&mut report, &mut context);
    }
    report
}

fn append_registers(report: &mut String, context: &CONTEXT) {
    let _ = writeln!(report, "\nregisters:");
    let _ = writeln!(
        report,
        "  RAX={:016X} RBX={:016X} RCX={:016X} RDX={:016X}",
        context.Rax, context.Rbx, context.Rcx, context.Rdx
    );
    let _ = writeln!(
        report,
        "  RSI={:016X} RDI={:016X} RBP={:016X} RSP={:016X}",
        context.Rsi, context.Rdi, context.Rbp, context.Rsp
    );
    let _ = writeln!(
        report,
        "  R8 ={:016X} R9 ={:016X} R10={:016X} R11={:016X}",
        context.R8, context.R9, context.R10, context.R11
    );
    let _ = writeln!(
        report,
        "  R12={:016X} R13={:016X} R14={:016X} R15={:016X}",
        context.R12, context.R13, context.R14, context.R15
    );
    let _ = writeln!(
        report,
        "  RIP={:016X} EFLAGS={:08X}",
        context.Rip, context.EFlags
    );
}

unsafe fn append_context_stack(report: &mut String, context: &mut CONTEXT) {
    let process = GetCurrentProcess();
    let thread = GetCurrentThread();
    let mut frame: STACKFRAME64 = std::mem::zeroed();
    frame.AddrPC.Offset = context.Rip;
    frame.AddrPC.Mode = AddrModeFlat;
    frame.AddrFrame.Offset = context.Rbp;
    frame.AddrFrame.Mode = AddrModeFlat;
    frame.AddrStack.Offset = context.Rsp;
    frame.AddrStack.Mode = AddrModeFlat;

    let _ = SymInitialize(process, std::ptr::null(), 1);
    let _ = writeln!(report, "\nstack_trace:");
    for index in 0..64 {
        let address = frame.AddrPC.Offset as usize;
        if address == 0 {
            break;
        }
        let location = module_location(address).unwrap_or_else(|| "<unknown>".to_owned());
        let _ = writeln!(report, "  #{index:02} {address:#018x} {location}");
        let previous = frame.AddrPC.Offset;
        if StackWalk64(
            u32::from(IMAGE_FILE_MACHINE_AMD64),
            process,
            thread,
            &mut frame,
            (context as *mut CONTEXT).cast(),
            None,
            Some(SymFunctionTableAccess64),
            Some(SymGetModuleBase64),
            None,
        ) == 0
            || frame.AddrPC.Offset == previous
        {
            break;
        }
    }
    let _ = SymCleanup(process);
}

unsafe fn append_current_stack(report: &mut String, frames_to_skip: u32) {
    let mut frames = [std::ptr::null_mut::<c_void>(); 64];
    let count = RtlCaptureStackBackTrace(
        frames_to_skip.saturating_add(1),
        frames.len() as u32,
        frames.as_mut_ptr(),
        std::ptr::null_mut(),
    ) as usize;
    let _ = writeln!(report, "stack_trace:");
    for (index, frame) in frames[..count].iter().enumerate() {
        let address = *frame as usize;
        let location = module_location(address).unwrap_or_else(|| "<unknown>".to_owned());
        let _ = writeln!(report, "  #{index:02} {address:#018x} {location}");
    }
}

fn module_location(address: usize) -> Option<String> {
    let pointer = address as *const ();
    let path = win32::module_path(pointer)?;
    let base = win32::module_base(pointer)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    Some(format!("{name}+0x{:X}", address.saturating_sub(base)))
}

fn emit_report(prefix: &str, summary: &str, report: &str) {
    let path = report_path(prefix);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let saved = std::fs::write(&path, report).is_ok();

    crate::console::error(summary);
    for line in report.lines() {
        crate::console::error(line);
    }
    if saved {
        crate::console::error(&format!("Crash report saved: {}", path.display()));
    } else {
        crate::console::error("Unable to save the crash report");
    }
}

fn report_path(prefix: &str) -> PathBuf {
    let base = PathBuf::from(crate::startup::executable_base_dir());
    base.join("mods")
        .join("crash")
        .join(format!("{prefix}_{}.log", timestamp()))
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}_{:03}", now.as_secs(), now.subsec_millis())
}

fn exception_name(code: u32) -> &'static str {
    match code {
        0xC000_0005 => "ACCESS_VIOLATION",
        0xC000_001D => "ILLEGAL_INSTRUCTION",
        0xC000_0094 => "INTEGER_DIVIDE_BY_ZERO",
        0xC000_00FD => "STACK_OVERFLOW",
        0xC000_0374 => "HEAP_CORRUPTION",
        0x8000_0003 => "BREAKPOINT",
        _ => "UNKNOWN",
    }
}
