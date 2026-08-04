use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::Sleep;

use super::log::{log_info, log_warn};
use super::seh::call_mod_on_frame;
use super::state::STATE;

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

static FRAME_THREAD_STARTED: AtomicBool = AtomicBool::new(false);
static FRAME_THREAD_RUNNING: AtomicBool = AtomicBool::new(false);
static FRAME_THREAD_HANDLE: Mutex<Option<SendHandle>> = Mutex::new(None);

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

pub fn start_if_needed() {
    let has_frame_mod = STATE
        .lock()
        .map(|state| state.mods.iter().any(|m| m.on_frame.is_some()))
        .unwrap_or(false);
    if !has_frame_mod {
        return;
    }

    if FRAME_THREAD_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    FRAME_THREAD_RUNNING.store(true, Ordering::SeqCst);
    unsafe {
        let handle = CreateThread(
            std::ptr::null(),
            0,
            Some(frame_thread),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );
        if handle.is_null() {
            FRAME_THREAD_RUNNING.store(false, Ordering::SeqCst);
            FRAME_THREAD_STARTED.store(false, Ordering::SeqCst);
            log_warn("Failed to start the module frame callback fallback thread");
            return;
        }
        if let Ok(mut h) = FRAME_THREAD_HANDLE.lock() {
            *h = Some(SendHandle(handle));
        }
    }
    log_info("Module frame callback fallback thread started with a 16 ms interval");
}

pub fn stop() {
    if !FRAME_THREAD_STARTED.load(Ordering::SeqCst) {
        return;
    }
    FRAME_THREAD_RUNNING.store(false, Ordering::SeqCst);
    unsafe {
        if let Ok(mut h) = FRAME_THREAD_HANDLE.lock() {
            if let Some(SendHandle(handle)) = h.take() {
                WaitForSingleObject(handle, 2000);
                CloseHandle(handle);
            }
        }
    }
    FRAME_THREAD_STARTED.store(false, Ordering::SeqCst);
}

unsafe extern "system" fn frame_thread(_param: *mut std::ffi::c_void) -> u32 {
    while FRAME_THREAD_RUNNING.load(Ordering::SeqCst) {
        tick();
        Sleep(16);
    }
    0
}

pub unsafe fn tick() {
    if super::hot_reload::is_reloading() {
        return;
    }

    let frame_mods: Vec<_> = STATE
        .lock()
        .map(|state| {
            state
                .mods
                .iter()
                .filter_map(|m| m.on_frame.map(|on_frame| (m.name.clone(), on_frame)))
                .collect()
        })
        .unwrap_or_default();

    for (name, on_frame) in frame_mods {
        call_mod_on_frame(&name, on_frame);
    }
}
