use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

use super::osd;

static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_TIME: AtomicU64 = AtomicU64::new(0);
static FREQ: OnceLock<u64> = OnceLock::new();

pub(crate) fn on_present(_device: *mut c_void) {
    let freq = *FREQ.get_or_init(|| {
        let mut freq = 0i64;
        let mut now = 0i64;
        // SAFETY: 两个 Win32 API 仅写入调用期间有效的栈变量
        unsafe {
            QueryPerformanceFrequency(&mut freq);
            QueryPerformanceCounter(&mut now);
        }
        LAST_TIME.store(now as u64, Ordering::Release);
        freq as u64
    });

    FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    let mut now = 0i64;
    // SAFETY: Win32 API 仅写入调用期间有效的栈变量
    unsafe { QueryPerformanceCounter(&mut now) };
    let now = now as u64;
    let previous = LAST_TIME.load(Ordering::Acquire);
    let elapsed = now.saturating_sub(previous);
    if freq > 0
        && elapsed >= freq
        && LAST_TIME
            .compare_exchange(previous, now, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        let count = FRAME_COUNT.swap(0, Ordering::Relaxed);
        let fps_x10 = (count as f64 * freq as f64 / elapsed as f64 * 10.0) as u32;
        osd::set_fps_x10(fps_x10);
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}
