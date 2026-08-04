use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

use crate::util::iat_hook::hook_iat;

use super::crash_dump::{self, stack_trace::append_current_stack_trace};
use super::log::{log_info, log_warn};

type TerminateProcessFn = unsafe extern "system" fn(*mut c_void, u32) -> i32;
type RaiseExceptionFn = unsafe extern "system" fn(u32, u32, u32, *const usize);
type ExitFn = unsafe extern "C" fn(i32) -> !;
type AbortFn = unsafe extern "C" fn() -> !;

static TERMINATE_PROCESS: AtomicUsize = AtomicUsize::new(0);
static RAISE_EXCEPTION: AtomicUsize = AtomicUsize::new(0);
static CRT_EXIT: AtomicUsize = AtomicUsize::new(0);
static CRT_IMMEDIATE_EXIT: AtomicUsize = AtomicUsize::new(0);
static CRT_ABORT: AtomicUsize = AtomicUsize::new(0);

const KERNEL32: &str = "kernel32.dll";
const CRT_RUNTIME: &str = "api-ms-win-crt-runtime-l1-1-0.dll";
const CPP_EXCEPTION: u32 = 0xE06D_7363;
const MSVC_THREAD_NAME_EXCEPTION: u32 = 0x406D_1388;

/// 只跟踪游戏主动结束路径，不改变原函数的参数或返回行为
pub unsafe fn install() {
    let game = GetModuleHandleA(std::ptr::null());
    if game.is_null() {
        log_warn("Unable to install game exit tracing because the game module is unavailable");
        return;
    }

    let mut installed = 0;
    installed += install_one(
        game as usize,
        KERNEL32,
        "TerminateProcess",
        hooked_terminate_process as *const (),
        &TERMINATE_PROCESS,
    );
    installed += install_one(
        game as usize,
        KERNEL32,
        "RaiseException",
        hooked_raise_exception as *const (),
        &RAISE_EXCEPTION,
    );
    installed += install_one(
        game as usize,
        CRT_RUNTIME,
        "exit",
        hooked_exit as *const (),
        &CRT_EXIT,
    );
    installed += install_one(
        game as usize,
        CRT_RUNTIME,
        "_exit",
        hooked_immediate_exit as *const (),
        &CRT_IMMEDIATE_EXIT,
    );
    installed += install_one(
        game as usize,
        CRT_RUNTIME,
        "abort",
        hooked_abort as *const (),
        &CRT_ABORT,
    );

    log_info(&format!(
        "Game exit tracing installed for {installed} entries"
    ));
}

unsafe fn install_one(
    game: usize,
    dll: &str,
    name: &str,
    replacement: *const (),
    original: &AtomicUsize,
) -> usize {
    let Some(address) = hook_iat(game, dll, name, replacement) else {
        return 0;
    };
    original.store(address as usize, Ordering::Release);
    1
}

unsafe extern "system" fn hooked_terminate_process(process: *mut c_void, exit_code: u32) -> i32 {
    log_exit_with_stack(&format!(
        "TerminateProcess(process={process:p}, code={exit_code:#010x})"
    ));
    let original: TerminateProcessFn = original(&TERMINATE_PROCESS);
    original(process, exit_code)
}

unsafe extern "system" fn hooked_raise_exception(
    code: u32,
    flags: u32,
    count: u32,
    arguments: *const usize,
) {
    // C++ throw 和旧式线程命名都通过 RaiseException 实现，不属于退出异常
    if code != CPP_EXCEPTION && code != MSVC_THREAD_NAME_EXCEPTION {
        log_warn(&format!(
            "Game raised an exception: code={code:#010x}, flags={flags:#010x}, parameters={count}"
        ));
    }
    let original: RaiseExceptionFn = original(&RAISE_EXCEPTION);
    original(code, flags, count, arguments);
}

unsafe extern "C" fn hooked_exit(code: i32) -> ! {
    report_exit(&format!("exit(code={code:#010x})"), code != 0);
    let original: ExitFn = original(&CRT_EXIT);
    original(code)
}

unsafe extern "C" fn hooked_immediate_exit(code: i32) -> ! {
    report_exit(&format!("_exit(code={code:#010x})"), code != 0);
    let original: ExitFn = original(&CRT_IMMEDIATE_EXIT);
    original(code)
}

unsafe extern "C" fn hooked_abort() -> ! {
    report_exit("abort()", true);
    let original: AbortFn = original(&CRT_ABORT);
    original()
}

unsafe fn report_exit(reason: &str, fatal: bool) {
    let stack = log_exit_with_stack(reason);
    if fatal {
        crash_dump::handle_deliberate_exit(&build_exit_report(reason, &stack));
    }
}

unsafe fn log_exit_with_stack(reason: &str) -> String {
    let mut stack = String::new();
    append_current_stack_trace(&mut stack, 2);
    log_warn(format!("Game is exiting: {reason}\n{stack}").trim_end());
    stack
}

fn build_exit_report(reason: &str, stack: &str) -> String {
    format!(
        "=== Chusan Exited Unexpectedly Nya... (>_<) ===\n\n\
         The game shut itself down instead of crashing, so there is no\n\
         exception record. The call stack below is where the exit came from.\n\n\
         reason: {reason}\n\n{stack}"
    )
}

unsafe fn original<T: Copy>(slot: &AtomicUsize) -> T {
    let address = slot.load(Ordering::Acquire);
    assert_ne!(address, 0, "exit trace original function is unavailable");
    std::mem::transmute_copy(&address)
}
