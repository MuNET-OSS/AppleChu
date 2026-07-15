mod controls;

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, SetBkColor, SetTextColor, FW_NORMAL, HBRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, MoveWindow,
    PostQuitMessage, RegisterClassExW, SendMessageW, SetWindowTextW, ShowWindow, TranslateMessage,
    CW_USEDEFAULT, MSG, SW_SHOW, WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC,
    WM_DESTROY, WM_SIZE, WNDCLASSEXW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
};

use super::theme::{current_theme, Theme, LIGHT};
use super::{wide, CRASH_DIR};

const ID_TEXT: isize = 100;
const ID_COPY: isize = 101;
const ID_OPEN: isize = 102;
const ID_EXIT: isize = 103;

const ES_MULTILINE: u32 = 0x0004;
const ES_AUTOVSCROLL: u32 = 0x0040;
const ES_READONLY: u32 = 0x0800;
const WM_SETFONT: u32 = 0x0030;
const EM_SETSEL: u32 = 0x00B1;
const WM_COPY: u32 = 0x0301;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

const MARGIN: i32 = 16;
const BTN_H: i32 = 36;
const BTN_GAP: i32 = 12;

static THEME: OnceLock<&'static Theme> = OnceLock::new();
static mut TEXT_HWND: HWND = std::ptr::null_mut();
static mut BG_BRUSH: HBRUSH = std::ptr::null_mut();
static mut TEXT_BRUSH: HBRUSH = std::ptr::null_mut();
static mut UI_FONT: windows_sys::Win32::Graphics::Gdi::HFONT = std::ptr::null_mut();

fn theme() -> &'static Theme {
    THEME.get().copied().unwrap_or(&LIGHT)
}

pub(super) unsafe fn run_window(body: &str) {
    let class_name = wide("ChusanCrashWindow");
    let hinstance = GetModuleHandleW(std::ptr::null());

    let selected_theme = current_theme();
    let _ = THEME.set(selected_theme);

    BG_BRUSH = CreateSolidBrush(selected_theme.bg);
    TEXT_BRUSH = CreateSolidBrush(selected_theme.text_bg);
    let font_name = wide("Cascadia Mono");
    UI_FONT = CreateFontW(
        18,
        0,
        0,
        0,
        FW_NORMAL as i32,
        0,
        0,
        0,
        1,
        0,
        0,
        4,
        0,
        font_name.as_ptr(),
    );
    if UI_FONT.is_null() {
        let fallback = wide("Consolas");
        UI_FONT = CreateFontW(
            18,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            1,
            0,
            0,
            4,
            0,
            fallback.as_ptr(),
        );
    }

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: BG_BRUSH,
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    RegisterClassExW(&wc);

    let title = wide("Chusan Crashed Nya~ (>_<)");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        760,
        600,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        return;
    }

    enable_dark_titlebar(hwnd, theme().dark);

    let text = controls::create_text_box(hwnd, hinstance, body);
    TEXT_HWND = text;
    controls::create_button(hwnd, hinstance, "Copy Log", ID_COPY);
    controls::create_button(hwnd, hinstance, "Open Folder", ID_OPEN);
    controls::create_button(hwnd, hinstance, "Exit", ID_EXIT);
    controls::layout(hwnd);

    ShowWindow(hwnd, SW_SHOW);

    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe fn enable_dark_titlebar(hwnd: HWND, dark: bool) {
    let dwmapi = wide("dwmapi.dll");
    let module = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(dwmapi.as_ptr());
    if module.is_null() {
        return;
    }
    type SetAttrFn = unsafe extern "system" fn(HWND, u32, *const core::ffi::c_void, u32) -> i32;
    let proc = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
        module,
        b"DwmSetWindowAttribute\0".as_ptr(),
    );
    if let Some(proc) = proc {
        let set_attr: SetAttrFn = std::mem::transmute(proc);
        let enabled: i32 = if dark { 1 } else { 0 };
        set_attr(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &enabled as *const i32 as *const _,
            4,
        );
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SIZE => {
            controls::layout(hwnd);
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let hdc = wparam as windows_sys::Win32::Graphics::Gdi::HDC;
            let selected_theme = theme();
            SetTextColor(hdc, selected_theme.text_fg);
            SetBkColor(hdc, selected_theme.text_bg);
            TEXT_BRUSH as LRESULT
        }
        WM_CTLCOLORBTN => BG_BRUSH as LRESULT,
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as isize;
            match id {
                ID_COPY if !TEXT_HWND.is_null() => {
                    SendMessageW(TEXT_HWND, EM_SETSEL, 0, -1isize as LPARAM);
                    SendMessageW(TEXT_HWND, WM_COPY, 0, 0);
                }
                ID_OPEN => open_crash_folder(),
                ID_EXIT => PostQuitMessage(0),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn open_crash_folder() {
    let Some(dir) = CRASH_DIR.get() else {
        return;
    };
    let verb = wide("open");
    windows_sys::Win32::UI::Shell::ShellExecuteW(
        std::ptr::null_mut(),
        verb.as_ptr(),
        dir.as_ptr(),
        std::ptr::null(),
        std::ptr::null(),
        SW_SHOW,
    );
}
