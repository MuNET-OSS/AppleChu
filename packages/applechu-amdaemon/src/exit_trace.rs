use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use applechu::iohook::hook_table::{hook_table_apply, HookSymbol};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetProcessId};

type ExitFn = unsafe extern "C" fn(i32) -> !;
type AbortFn = unsafe extern "C" fn() -> !;
type ExitProcessFn = unsafe extern "system" fn(u32) -> !;
type TerminateProcessFn = unsafe extern "system" fn(HANDLE, u32) -> i32;

static CRT_EXIT: AtomicUsize = AtomicUsize::new(0);
static CRT_IMMEDIATE_EXIT: AtomicUsize = AtomicUsize::new(0);
static CRT_ABORT: AtomicUsize = AtomicUsize::new(0);
static CRT_TERMINATE_PROCESS: AtomicUsize = AtomicUsize::new(0);
static EXIT_PROCESS: AtomicUsize = AtomicUsize::new(0);
static TERMINATE_PROCESS: AtomicUsize = AtomicUsize::new(0);
static EXIT_REPORTED: AtomicBool = AtomicBool::new(false);

/// 记录 AM Daemon 主动退出的路径，保持原函数的参数和返回行为
pub(crate) unsafe fn install() {
    let executable = GetModuleHandleW(std::ptr::null());
    if executable.is_null() {
        crate::console::warn("Unable to install AM Daemon exit tracing");
        return;
    }

    let mut installed = 0;
    installed += install_one(
        executable,
        "msvcr110.dll",
        "exit",
        hooked_exit as *const (),
        &CRT_EXIT,
    );
    installed += install_one(
        executable,
        "msvcr110.dll",
        "_exit",
        hooked_immediate_exit as *const (),
        &CRT_IMMEDIATE_EXIT,
    );
    installed += install_one(
        executable,
        "msvcr110.dll",
        "abort",
        hooked_abort as *const (),
        &CRT_ABORT,
    );
    installed += install_one(
        executable,
        "msvcr110.dll",
        "__crtTerminateProcess",
        hooked_crt_terminate_process as *const (),
        &CRT_TERMINATE_PROCESS,
    );
    installed += install_one(
        executable,
        "kernel32.dll",
        "ExitProcess",
        hooked_exit_process as *const (),
        &EXIT_PROCESS,
    );
    installed += install_one(
        executable,
        "kernel32.dll",
        "TerminateProcess",
        hooked_terminate_process as *const (),
        &TERMINATE_PROCESS,
    );

    crate::console::info(&format!(
        "AM Daemon exit tracing installed for {installed} entries"
    ));
}

unsafe fn install_one(
    executable: *mut c_void,
    module: &str,
    name: &'static str,
    replacement: *const (),
    original: &AtomicUsize,
) -> usize {
    let mut original_ptr = std::ptr::null();
    let symbols = [HookSymbol {
        name,
        patch: replacement,
        original: &mut original_ptr,
    }];
    let patched = hook_table_apply(executable, module, &symbols);
    if !original_ptr.is_null() {
        original.store(original_ptr as usize, Ordering::Release);
    }
    patched
}

unsafe extern "C" fn hooked_exit(code: i32) -> ! {
    EXIT_REPORTED.store(true, Ordering::Release);
    log_exit("exit", code);
    original(&CRT_EXIT)(code)
}

unsafe extern "C" fn hooked_immediate_exit(code: i32) -> ! {
    EXIT_REPORTED.store(true, Ordering::Release);
    log_exit("_exit", code);
    original(&CRT_IMMEDIATE_EXIT)(code)
}

unsafe extern "C" fn hooked_abort() -> ! {
    EXIT_REPORTED.store(true, Ordering::Release);
    crate::console::error("AM Daemon terminated through abort()");
    crate::crash::report_exit("abort", 3);
    original_abort(&CRT_ABORT)()
}

unsafe extern "C" fn hooked_crt_terminate_process(code: i32) -> ! {
    EXIT_REPORTED.store(true, Ordering::Release);
    log_exit("__crtTerminateProcess", code);
    original(&CRT_TERMINATE_PROCESS)(code)
}

unsafe extern "system" fn hooked_exit_process(code: u32) -> ! {
    if !EXIT_REPORTED.swap(true, Ordering::AcqRel) {
        log_exit("ExitProcess", code as i32);
    }
    original_exit_process(&EXIT_PROCESS)(code)
}

unsafe extern "system" fn hooked_terminate_process(process: HANDLE, code: u32) -> i32 {
    if GetProcessId(process) == GetCurrentProcessId() && !EXIT_REPORTED.swap(true, Ordering::AcqRel)
    {
        log_exit("TerminateProcess", code as i32);
    }
    original_terminate_process(&TERMINATE_PROCESS)(process, code)
}

fn log_exit(source: &str, code: i32) {
    let message = format!("AM Daemon is exiting: source={source}, code={code:#010x}");
    if code == 0 {
        crate::console::info(&message);
    } else {
        crate::console::warn(&message);
        crate::crash::report_exit(source, code);
    }
}

unsafe fn original(slot: &AtomicUsize) -> ExitFn {
    let address = slot.load(Ordering::Acquire);
    assert_ne!(address, 0, "AM Daemon exit trace original is unavailable");
    std::mem::transmute(address)
}

unsafe fn original_abort(slot: &AtomicUsize) -> AbortFn {
    let address = slot.load(Ordering::Acquire);
    assert_ne!(address, 0, "AM Daemon abort trace original is unavailable");
    std::mem::transmute(address)
}

unsafe fn original_exit_process(slot: &AtomicUsize) -> ExitProcessFn {
    let address = slot.load(Ordering::Acquire);
    assert_ne!(address, 0, "AM Daemon ExitProcess original is unavailable");
    std::mem::transmute(address)
}

unsafe fn original_terminate_process(slot: &AtomicUsize) -> TerminateProcessFn {
    let address = slot.load(Ordering::Acquire);
    assert_ne!(
        address, 0,
        "AM Daemon TerminateProcess original is unavailable"
    );
    std::mem::transmute(address)
}
