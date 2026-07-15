use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::config::Config;
use crate::gfx::WindowConfig;
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat_all_modules;

const WS_BORDER: u32 = 0x0080_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_POPUP: u32 = 0x8000_0000;
const SW_RESTORE: i32 = 9;
const SW_SHOW: i32 = 5;
const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;

type CreateWindowExAFn = unsafe extern "system" fn(
    u32,
    *const u8,
    *const u8,
    u32,
    i32,
    i32,
    i32,
    i32,
    usize,
    usize,
    usize,
    *const c_void,
) -> usize;
type ShowWindowFn = unsafe extern "system" fn(usize, i32) -> i32;

static ORIG_CREATE_WINDOW: AtomicUsize = AtomicUsize::new(0);
static ORIG_SHOW_WINDOW: AtomicUsize = AtomicUsize::new(0);
static WINDOWED: AtomicBool = AtomicBool::new(false);
static FRAMED: AtomicBool = AtomicBool::new(true);

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32")]
extern "system" {
    fn AdjustWindowRect(rect: *mut Rect, style: u32, menu: i32) -> i32;
}

pub fn init(api: &Api, config: &Config) {
    let Some(config) = config.section::<WindowConfig>() else {
        return;
    };
    if !config.enabled || !config.windowed {
        return;
    }

    WINDOWED.store(true, Ordering::SeqCst);
    FRAMED.store(config.framed, Ordering::SeqCst);

    let mut installed = false;
    unsafe {
        if let Some(original) = hook_iat_all_modules(
            "user32.dll",
            "CreateWindowExA",
            hooked_create_window_ex_a as *const (),
        ) {
            ORIG_CREATE_WINDOW.store(original as usize, Ordering::SeqCst);
            installed = true;
        }

        if let Some(original) =
            hook_iat_all_modules("user32.dll", "ShowWindow", hooked_show_window as *const ())
        {
            ORIG_SHOW_WINDOW.store(original as usize, Ordering::SeqCst);
        }
    }

    if installed {
        api.log_info(&format!(
            "gfx: window hook installed (framed={})",
            FRAMED.load(Ordering::SeqCst)
        ));
    } else {
        api.log_warn("gfx: CreateWindowExA import not found");
    }
}

unsafe extern "system" fn hooked_create_window_ex_a(
    ex_style: u32,
    class_name: *const u8,
    window_name: *const u8,
    mut style: u32,
    mut x: i32,
    mut y: i32,
    mut width: i32,
    mut height: i32,
    parent: usize,
    menu: usize,
    instance: usize,
    param: *const c_void,
) -> usize {
    let original_addr = ORIG_CREATE_WINDOW.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }

    if WINDOWED.load(Ordering::SeqCst) {
        style = if FRAMED.load(Ordering::SeqCst) {
            WS_BORDER | WS_CAPTION | WS_MINIMIZEBOX | WS_SYSMENU
        } else {
            WS_POPUP
        };

        let mut rect = Rect {
            left: if x == CW_USEDEFAULT { 0 } else { x },
            top: if y == CW_USEDEFAULT { 0 } else { y },
            right: 0,
            bottom: 0,
        };
        rect.right = rect.left + width;
        rect.bottom = rect.top + height;
        AdjustWindowRect(&mut rect, style, i32::from(menu != 0));

        if x != CW_USEDEFAULT {
            x = rect.left;
        }
        if y != CW_USEDEFAULT {
            y = rect.top;
        }
        width = rect.right - rect.left;
        height = rect.bottom - rect.top;
    }

    let original: CreateWindowExAFn = std::mem::transmute(original_addr);
    original(
        ex_style,
        class_name,
        window_name,
        style,
        x,
        y,
        width,
        height,
        parent,
        menu,
        instance,
        param,
    )
}

unsafe extern "system" fn hooked_show_window(hwnd: usize, mut cmd_show: i32) -> i32 {
    let original_addr = ORIG_SHOW_WINDOW.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }

    if !FRAMED.load(Ordering::SeqCst) && cmd_show == SW_RESTORE {
        cmd_show = SW_SHOW;
    }

    let original: ShowWindowFn = std::mem::transmute(original_addr);
    original(hwnd, cmd_show)
}
