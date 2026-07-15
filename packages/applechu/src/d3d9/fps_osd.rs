use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use super::osd;

static mut INITIALIZED: bool = false;
static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
static mut LAST_TIME: u64 = 0;
static mut FREQ: u64 = 0;

pub unsafe extern "C" fn on_present(_device: *mut c_void) {
    if !INITIALIZED {
        INITIALIZED = true;
        let mut freq = 0i64;
        let mut now = 0i64;
        QueryPerformanceFrequency(&mut freq);
        QueryPerformanceCounter(&mut now);
        FREQ = freq as u64;
        LAST_TIME = now as u64;
    }

    FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    let mut now = 0i64;
    QueryPerformanceCounter(&mut now);
    let now = now as u64;
    let elapsed = now.saturating_sub(LAST_TIME);
    if FREQ > 0 && elapsed >= FREQ {
        let count = FRAME_COUNT.swap(0, Ordering::Relaxed);
        let fps_x10 = (count as f64 * FREQ as f64 / elapsed as f64 * 10.0) as u32;
        osd::set_fps_x10(fps_x10);
        LAST_TIME = now;
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}
