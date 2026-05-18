use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

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

#[repr(C)]
struct WndClassA {
    style: u32,
    wnd_proc: unsafe extern "system" fn(usize, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: usize,
    icon: usize,
    cursor: usize,
    background: usize,
    menu_name: *const u8,
    class_name: *const u8,
}

#[repr(C)]
struct PaintStruct {
    hdc: *mut c_void,
    erase: i32,
    rc_paint: Rect,
    restore: i32,
    inc_update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

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

        // BGR: R=62 G=62 B=66 → 0x423E3E
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
    fn GetModuleHandleA(name: *const u8) -> usize;
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassA(wc: *const WndClassA) -> u16;
    fn CreateWindowExA(
        ex_style: u32, class: *const u8, name: *const u8, style: u32,
        x: i32, y: i32, w: i32, h: i32,
        parent: usize, menu: usize, instance: usize, param: *const u8,
    ) -> usize;
    fn ShowWindow(hwnd: usize, cmd: i32) -> i32;
    fn SetLayeredWindowAttributes(hwnd: usize, key: u32, alpha: u8, flags: u32) -> i32;
    fn DefWindowProcA(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize;
    fn BeginPaint(hwnd: usize, ps: *mut PaintStruct) -> *mut c_void;
    fn EndPaint(hwnd: usize, ps: *const PaintStruct) -> i32;
    fn GetClientRect(hwnd: usize, rect: *mut Rect) -> i32;
    fn InvalidateRect(hwnd: usize, rect: *const c_void, erase: i32) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateSolidBrush(color: u32) -> *mut c_void;
    fn CreateFontA(
        h: i32, w: i32, esc: i32, orient: i32, weight: i32,
        italic: u32, underline: u32, strikeout: u32, charset: u32,
        out_prec: u32, clip_prec: u32, quality: u32, pitch: u32,
        face: *const u8,
    ) -> *mut c_void;
    fn SelectObject(hdc: *mut c_void, obj: *mut c_void) -> *mut c_void;
    fn DeleteObject(obj: *mut c_void) -> i32;
    fn SetBkMode(hdc: *mut c_void, mode: i32) -> i32;
    fn SetTextColor(hdc: *mut c_void, color: u32) -> u32;
    fn FillRect(hdc: *mut c_void, rect: *const Rect, brush: *mut c_void) -> i32;
    fn DrawTextW(hdc: *mut c_void, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
}
