use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::hooks::autoplay::{
    BeginPaint, CreateFontA, CreateSolidBrush, CreateWindowExA, DefWindowProcA, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetClientRect, GetModuleHandleA, InvalidateRect, RegisterClassA,
    SelectObject, SetBkMode, SetLayeredWindowAttributes, SetTextColor, ShowWindow,
};
use crate::hooks::autoplay::{PaintStruct, Rect, WndClassA};

const WS_POPUP: u32 = 0x80000000;
const WS_EX_TOPMOST: u32 = 0x00000008;
const WS_EX_LAYERED: u32 = 0x00080000;
const WS_EX_TRANSPARENT: u32 = 0x00000020;
const WS_EX_TOOLWINDOW: u32 = 0x00000080;
const LWA_ALPHA: u32 = 0x00000002;
const SW_SHOWNOACTIVATE: i32 = 4;
const WM_PAINT: u32 = 0x000F;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u32 = 1;
const ANTIALIASED_QUALITY: u32 = 4;
const DT_CENTER: u32 = 0x01;
const DT_VCENTER: u32 = 0x04;
const DT_SINGLELINE: u32 = 0x20;

const OSD_WIDTH: i32 = 140;
const OSD_HEIGHT: i32 = 30;
const OSD_MARGIN: i32 = 12;

static mut OSD_HWND: usize = 0;
static mut INITIALIZED: bool = false;
static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
static FPS_X10: AtomicU32 = AtomicU32::new(0);
static mut LAST_TIME: u64 = 0;
static mut FREQ: u64 = 0;

pub unsafe extern "C" fn on_end_scene(_device: *mut c_void) {
    if !INITIALIZED {
        INITIALIZED = true;
        let mut freq = 0i64;
        let mut now = 0i64;
        QueryPerformanceFrequency(&mut freq);
        QueryPerformanceCounter(&mut now);
        FREQ = freq as u64;
        LAST_TIME = now as u64;
        create_osd();
    }

    FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    let mut now = 0i64;
    QueryPerformanceCounter(&mut now);
    let now = now as u64;
    let elapsed = now - LAST_TIME;
    if FREQ > 0 && elapsed >= FREQ {
        let count = FRAME_COUNT.swap(0, Ordering::Relaxed);
        let fps_x10 = (count as f64 * FREQ as f64 / elapsed as f64 * 10.0) as u32;
        FPS_X10.store(fps_x10, Ordering::Relaxed);
        LAST_TIME = now;
    }

    if OSD_HWND != 0 {
        InvalidateRect(OSD_HWND, ptr::null(), 1);
    }
}

unsafe fn create_osd() {
    let instance = GetModuleHandleA(ptr::null());
    let wc = WndClassA {
        style: 0,
        wnd_proc: osd_wndproc,
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: 0,
        cursor: 0,
        background: 0,
        menu_name: ptr::null(),
        class_name: b"AppleChuFpsOSD\0".as_ptr(),
    };
    RegisterClassA(&wc);
    OSD_HWND = CreateWindowExA(
        WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
        b"AppleChuFpsOSD\0".as_ptr(),
        ptr::null(),
        WS_POPUP,
        OSD_MARGIN, OSD_MARGIN, OSD_WIDTH, OSD_HEIGHT,
        0, 0, instance, ptr::null(),
    );
    if OSD_HWND != 0 {
        SetLayeredWindowAttributes(OSD_HWND, 0, 153, LWA_ALPHA);
        ShowWindow(OSD_HWND, SW_SHOWNOACTIVATE);
    }
}

unsafe extern "system" fn osd_wndproc(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_PAINT {
        let mut ps: PaintStruct = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rect: Rect = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);

        let bg_brush = CreateSolidBrush(0x00423E3E);
        FillRect(hdc, &rect, bg_brush);
        DeleteObject(bg_brush);

        SetBkMode(hdc, 1);
        SetTextColor(hdc, 0x00FFFFFF);

        let font = CreateFontA(
            18, 0, 0, 0, FW_BOLD, 0, 0, 0,
            DEFAULT_CHARSET, 0, 0, ANTIALIASED_QUALITY, 0,
            b"Segoe UI\0".as_ptr(),
        );
        let old = SelectObject(hdc, font);

        let fps_x10 = FPS_X10.load(Ordering::Relaxed);
        let text = format!("{}.{} FPS\0", fps_x10 / 10, fps_x10 % 10);
        let wide: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(hdc, wide.as_ptr(), wide.len() as i32 - 1, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        SelectObject(hdc, old);
        DeleteObject(font);
        EndPaint(hwnd, &ps);
        return 0;
    }
    DefWindowProcA(hwnd, msg, wp, lp)
}

#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}
