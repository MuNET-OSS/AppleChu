use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::Sleep;

use super::reload_all_mods;
use crate::proxy::loader::log::{log_info, log_warn};
use crate::proxy::loader::state::STATE;

static MONITOR_STARTED: AtomicBool = AtomicBool::new(false);
static MONITOR_RUNNING: AtomicBool = AtomicBool::new(false);

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

static MONITOR_THREAD_HANDLE: Mutex<Option<SendHandle>> = Mutex::new(None);

extern "system" {
    fn CreateThread(
        attrs: *const std::ffi::c_void,
        stack_size: usize,
        start: Option<unsafe extern "system" fn(*mut std::ffi::c_void) -> u32>,
        param: *mut std::ffi::c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, milliseconds: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> i32;
}

pub fn start_monitor() {
    if MONITOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    MONITOR_RUNNING.store(true, Ordering::SeqCst);
    unsafe {
        let handle = CreateThread(
            std::ptr::null(),
            0,
            Some(reload_flag_thread),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );
        if handle.is_null() {
            MONITOR_RUNNING.store(false, Ordering::SeqCst);
            MONITOR_STARTED.store(false, Ordering::SeqCst);
            log_warn("failed to start reload.flag monitor thread");
            return;
        }
        if let Ok(mut thread) = MONITOR_THREAD_HANDLE.lock() {
            *thread = Some(SendHandle(handle));
        }
    }
    log_info("reload.flag monitor thread started");
}

pub fn stop_monitor() {
    if !MONITOR_STARTED.load(Ordering::SeqCst) {
        return;
    }
    MONITOR_RUNNING.store(false, Ordering::SeqCst);
    unsafe {
        if let Ok(mut thread) = MONITOR_THREAD_HANDLE.lock() {
            if let Some(SendHandle(handle)) = thread.take() {
                WaitForSingleObject(handle, 2000);
                CloseHandle(handle);
            }
        }
    }
    MONITOR_STARTED.store(false, Ordering::SeqCst);
}

unsafe extern "system" fn reload_flag_thread(_param: *mut std::ffi::c_void) -> u32 {
    while MONITOR_RUNNING.load(Ordering::SeqCst) {
        poll_reload_flag();
        Sleep(500);
    }
    0
}

pub(super) unsafe fn poll_reload_flag() {
    let base_dir = STATE
        .lock()
        .map(|state| state.base_dir.clone())
        .unwrap_or_default();
    if base_dir.is_empty() {
        return;
    }
    let flag_path = format!("{base_dir}\\mods\\reload.flag");
    if !Path::new(&flag_path).exists() {
        return;
    }

    log_info("reload.flag detected; reloading all mods");
    let _ = reload_all_mods();
    if let Err(err) = std::fs::remove_file(&flag_path) {
        log_warn(&format!("failed to remove reload.flag: {err}"));
    }
}
