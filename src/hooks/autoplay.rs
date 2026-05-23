use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::util::api::Api;

static ENABLED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static WAS_USED: AtomicBool = AtomicBool::new(false);

static mut API_HANDLE: Option<Api> = None;
static mut STATE_PTR: usize = 0;
static mut DEMO_OFF: u32 = 0;
static mut PRESET_OFF: u32 = 0;
static mut OSD_HWND: usize = 0;
static mut JUDGE_ADDR: usize = 0;
static mut ORIG_JUDGE: usize = 0;

#[link(name = "kernel32")]
extern "system" {
    pub fn CreateThread(
        attrs: *const c_void,
        stack_size: usize,
        start: Option<unsafe extern "system" fn(*mut c_void) -> u32>,
        param: *const c_void,
        flags: u32,
        id: *mut u32,
    ) -> usize;
    pub fn Sleep(ms: u32);
    pub fn GetModuleHandleA(module_name: *const u8) -> usize;
}

#[link(name = "user32")]
extern "system" {
    pub fn GetAsyncKeyState(vkey: i32) -> i16;
    pub fn CreateWindowExA(
        ex_style: u32,
        class_name: *const u8,
        window_name: *const u8,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: usize,
        menu: usize,
        instance: usize,
        param: *const c_void,
    ) -> usize;
    pub fn RegisterClassA(wc: *const WndClassA) -> u16;
    pub fn DefWindowProcA(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize;
    pub fn ShowWindow(hwnd: usize, cmd: i32) -> i32;
    pub fn SetLayeredWindowAttributes(hwnd: usize, cr_key: u32, alpha: u8, flags: u32) -> i32;
    pub fn BeginPaint(hwnd: usize, ps: *mut PaintStruct) -> usize;
    pub fn EndPaint(hwnd: usize, ps: *const PaintStruct) -> i32;
    pub fn GetClientRect(hwnd: usize, rect: *mut Rect) -> i32;
    pub fn InvalidateRect(hwnd: usize, rect: *const Rect, erase: i32) -> i32;
    pub fn FillRect(hdc: usize, rect: *const Rect, brush: usize) -> i32;
    pub fn DrawTextW(hdc: usize, text: *const u16, count: i32, rect: *mut Rect, format: u32) -> i32;
    pub fn GetSystemMetrics(index: i32) -> i32;
    pub fn SetWindowPos(hwnd: usize, insert_after: usize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    pub fn PeekMessageA(msg: *mut Msg, hwnd: usize, min: u32, max: u32, remove: u32) -> i32;
    pub fn TranslateMessage(msg: *const Msg) -> i32;
    pub fn DispatchMessageA(msg: *const Msg) -> isize;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn SetBkMode(hdc: usize, mode: i32) -> i32;
    pub fn SetTextColor(hdc: usize, color: u32) -> u32;
    pub fn CreateFontA(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strikeout: u32,
        charset: u32,
        out_prec: u32,
        clip_prec: u32,
        quality: u32,
        pitch: u32,
        face: *const u8,
    ) -> usize;
    pub fn SelectObject(hdc: usize, obj: usize) -> usize;
    pub fn DeleteObject(obj: usize) -> i32;
    pub fn CreateSolidBrush(color: u32) -> usize;
}

const DT_CENTER: u32 = 0x01;
const DT_VCENTER: u32 = 0x04;
const DT_SINGLELINE: u32 = 0x20;

#[repr(C)]
pub struct WndClassA {
    pub style: u32,
    pub wnd_proc: unsafe extern "system" fn(usize, u32, usize, isize) -> isize,
    pub cls_extra: i32,
    pub wnd_extra: i32,
    pub instance: usize,
    pub icon: usize,
    pub cursor: usize,
    pub background: usize,
    pub menu_name: *const u8,
    pub class_name: *const u8,
}

#[repr(C)]
pub struct PaintStruct {
    pub hdc: usize,
    pub erase: i32,
    pub rc_paint: Rect,
    pub restore: i32,
    pub inc_update: i32,
    pub rgb_reserved: [u8; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct Msg {
    pub hwnd: usize,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt_x: i32,
    pub pt_y: i32,
}

const WM_PAINT: u32 = 0x000F;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const LWA_ALPHA: u32 = 0x02;
const SW_SHOWNOACTIVATE: i32 = 8;
const SW_HIDE: i32 = 0;
const FW_BOLD: i32 = 700;
const DEFAULT_CHARSET: u32 = 1;
const ANTIALIASED_QUALITY: u32 = 4;
const PM_REMOVE: u32 = 0x0001;
const VK_HOME: i32 = 0x24;

pub fn init(api: &Api, _config: &Config) {
    unsafe { API_HANDLE = Some(*api) };
    RUNNING.store(true, Ordering::Relaxed);

    let text_base = api.text_base();
    let text_size = api.text_size();
    let game_base = api.game_base();
    let game_size = api.game_size();
    if text_base == 0 || text_size == 0 || game_base == 0 || game_size == 0 {
        api.log_error("Autoplay 初始化失败: 游戏模块信息无效");
        return;
    }

    let demo_checker = find_demo_checker(api, text_base, text_size);
    if demo_checker == 0 {
        api.log_error("Autoplay 初始化失败: demo checker 未找到");
        return;
    }
    let Some(demo_off) = read_u32(api, demo_checker + 2) else {
        api.log_error("Autoplay 初始化失败: demo 偏移读取失败");
        return;
    };
    let Some(preset_off) = read_u32(api, demo_checker + 11) else {
        api.log_error("Autoplay 初始化失败: preset 偏移读取失败");
        return;
    };
    unsafe {
        DEMO_OFF = demo_off;
        PRESET_OFF = preset_off;
    }
    api.log_info(&format!(
        "Autoplay demo checker @ 0x{demo_checker:08X}, 偏移 +0x{demo_off:X}/+0x{preset_off:X}"
    ));

    let judge = find_judge_check(api, text_base, text_size);
    if judge == 0 {
        api.log_error("Autoplay 初始化失败: JudgeTapChecker::check 未找到");
        return;
    }
    unsafe { JUDGE_ADDR = judge };
    api.log_info(&format!("Autoplay JudgeTapChecker::check @ 0x{judge:08X}"));

    let state_ptr = find_state_ptr(api, text_base, text_size, demo_checker, game_base, game_size);
    if state_ptr == 0 {
        api.log_error("Autoplay 初始化失败: 全局游戏状态指针未找到");
        return;
    }
    unsafe { STATE_PTR = state_ptr };
    api.log_info(&format!("Autoplay game state ptr @ 0x{state_ptr:08X}"));

    let Some(trampoline) = api.hook_create(judge, hooked_judge_check as *const () as usize) else {
        api.log_error("Autoplay 初始化失败: judge hook 创建失败");
        return;
    };
    unsafe { ORIG_JUDGE = trampoline };
    if !api.hook_enable(judge) {
        api.log_error("Autoplay 初始化失败: judge hook 启用失败");
        return;
    }

    unsafe {
        CreateThread(ptr::null(), 0, Some(demo_write_thread), ptr::null(), 0, ptr::null_mut());
        CreateThread(ptr::null(), 0, Some(hotkey_thread), ptr::null(), 0, ptr::null_mut());
    }
    api.log_info("Autoplay enabled: toggle with Home key, writes demo/preset flag when on");
}

pub fn shutdown() {
    RUNNING.store(false, Ordering::Relaxed);
    ENABLED.store(false, Ordering::Relaxed);
    unsafe {
        if let Some(api) = API_HANDLE {
            if JUDGE_ADDR != 0 {
                api.hook_disable(JUDGE_ADDR);
                api.hook_remove(JUDGE_ADDR);
                JUDGE_ADDR = 0;
            }
            api.log_info("Autoplay cleaned up");
        }
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn was_used() -> bool {
    WAS_USED.load(Ordering::Relaxed)
}

pub fn reset_was_used() {
    WAS_USED.store(false, Ordering::Relaxed);
}

fn find_demo_checker(api: &Api, text_base: usize, text_size: u32) -> usize {
    // demo checker: 读取 demo flag 后继续读取 preset flag，用于提取结构体偏移。
    let sig = [
        0x80, 0xB9, 0, 0, 0, 0, 0x00, 0x74, 0x00, 0x83, 0xB9, 0, 0, 0, 0, 0x00, 0x0F, 0x95,
        0xC0, 0xC3, 0x32, 0xC0, 0xC3,
    ];
    api.aob_scan(text_base, text_size, &sig, "xx????xx?xx????xxxxxxxx")
}

fn find_judge_check(api: &Api, text_base: usize, text_size: u32) -> usize {
    // MOVSS XMM0,[ECX+4]; MOVSS XMM2,[ESP+4]; COMISS; JBE; OR EAX,-1; RET 8
    // 这段在 JudgeTapChecker::check 内部，找到后需要回溯找函数入口
    let sig: [u8; 22] = [
        0xF3, 0x0F, 0x10, 0x41, 0x04, 0xF3, 0x0F, 0x10, 0x54, 0x24, 0x04, 0x0F, 0x2F, 0xC2,
        0x76, 0x00, 0x83, 0xC8, 0xFF, 0xC2, 0x08, 0x00,
    ];
    let found = api.aob_scan(text_base, text_size, &sig, "xxxxxxxxxxxxxxx?xxxxxx");
    if found == 0 {
        return 0;
    }
    // 往前回溯找函数入口 (PUSH EBP; MOV EBP,ESP 或 PUSH -1 序言)
    for back in 1..0x200usize {
        if found < back + text_base {
            break;
        }
        let addr = found - back;
        let mut buf = [0u8; 5];
        if !api.mem_read(addr, &mut buf) {
            continue;
        }
        // 55 8B EC 6A FF = push ebp; mov ebp,esp; push -1
        if buf[0] == 0x55 && buf[1] == 0x8B && buf[2] == 0xEC && buf[3] == 0x6A && buf[4] == 0xFF {
            return addr;
        }
        // 55 8B EC 83 EC = push ebp; mov ebp,esp; sub esp,...
        if buf[0] == 0x55 && buf[1] == 0x8B && buf[2] == 0xEC && buf[3] == 0x83 && buf[4] == 0xEC {
            return addr;
        }
    }
    found
}

fn find_state_ptr(
    api: &Api,
    text_base: usize,
    text_size: u32,
    demo_checker: usize,
    game_base: usize,
    game_size: u32,
) -> usize {
    let mut text = vec![0; text_size as usize];
    if !api.mem_read(text_base, &mut text) {
        return 0;
    }
    let demo_off = unsafe { DEMO_OFF };
    let game_end = game_base.saturating_add(game_size as usize);

    for i in 0..text.len().saturating_sub(7) {
        let addr = text_base + i;
        if (demo_checker..demo_checker + 24).contains(&addr) || text[i] != 0x80 {
            continue;
        }
        let modrm = text[i + 1];
        if !(0xB8..=0xBF).contains(&modrm) || modrm == 0xBC {
            continue;
        }
        if read_le_u32(&text, i + 2) != Some(demo_off) || text[i + 6] != 0 {
            continue;
        }

        let rm = (modrm & 7) as usize;
        let start = i.saturating_sub(300);
        for q in (start..i.saturating_sub(4)).rev() {
            if rm == 0 && text[q] == 0xA1 {
                if let Some(target) = read_le_u32(&text, q + 1).map(|value| value as usize) {
                    if (game_base..game_end).contains(&target) {
                        return target;
                    }
                }
            }
            if q + 6 <= text.len() && text[q] == 0x8B && text[q + 1] == ((rm << 3) | 0x05) as u8 {
                if let Some(target) = read_le_u32(&text, q + 2).map(|value| value as usize) {
                    if (game_base..game_end).contains(&target) {
                        return target;
                    }
                }
            }
        }
    }
    0
}

unsafe extern "fastcall" fn hooked_judge_check(
    ecx: *mut c_void,
    edx: *mut c_void,
    td: f32,
    input: u8,
) -> i32 {
    if ENABLED.load(Ordering::Relaxed) {
        write_demo_flags();
    }

    let orig: unsafe extern "fastcall" fn(*mut c_void, *mut c_void, f32, u8) -> i32 =
        std::mem::transmute(ORIG_JUDGE);
    orig(ecx, edx, td, input)
}

unsafe extern "system" fn demo_write_thread(_: *mut c_void) -> u32 {
    let mut was_enabled = false;
    while RUNNING.load(Ordering::Relaxed) {
        let enabled = ENABLED.load(Ordering::Relaxed);
        if enabled {
            write_demo_flags();
            if !was_enabled {
                log_info("Autoplay ON: writing demo/preset flag");
            }
        } else if was_enabled {
            clear_demo_flag();
            log_info("Autoplay OFF");
        }
        was_enabled = enabled;
        Sleep(16);
    }
    clear_demo_flag();
    0
}

unsafe extern "system" fn hotkey_thread(_: *mut c_void) -> u32 {
    osd_create();
    let mut msg: Msg = std::mem::zeroed();
    while RUNNING.load(Ordering::Relaxed) {
        while PeekMessageA(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageA(&msg);
        }
        if GetAsyncKeyState(VK_HOME) & 1 != 0 {
            let new_state = !ENABLED.load(Ordering::Relaxed);
            ENABLED.store(new_state, Ordering::Relaxed);
            if new_state {
                WAS_USED.store(true, Ordering::Relaxed);
            }
        }
        osd_update();
        Sleep(50);
    }
    osd_show(false);
    0
}

unsafe fn osd_update() {
    let is_on = ENABLED.load(Ordering::Relaxed);
    let was = WAS_USED.load(Ordering::Relaxed);
    if is_on || was {
        osd_show(true);
        osd_reposition();
        InvalidateRect(OSD_HWND, ptr::null(), 1);
    } else {
        osd_show(false);
    }
}

unsafe fn write_demo_flags() {
    let Some(api) = API_HANDLE else {
        return;
    };
    let Some(state) = read_usize(&api, STATE_PTR) else {
        return;
    };
    if state == 0 {
        return;
    }
    let _ = api.mem_write(state + DEMO_OFF as usize, &[1]);
    let _ = api.mem_write(state + PRESET_OFF as usize, &1u32.to_le_bytes());
}

unsafe fn clear_demo_flag() {
    let Some(api) = API_HANDLE else {
        return;
    };
    let Some(state) = read_usize(&api, STATE_PTR) else {
        return;
    };
    if state != 0 {
        let _ = api.mem_write(state + DEMO_OFF as usize, &[0]);
    }
}

const OSD_WIDTH: i32 = 220;
const OSD_HEIGHT: i32 = 36;
const OSD_MARGIN: i32 = 20;

unsafe extern "system" fn osd_wndproc(hwnd: usize, msg: u32, wp: usize, lp: isize) -> isize {
    if msg == WM_PAINT {
        let mut ps: PaintStruct = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rect: Rect = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);

        // GDI 使用 BGR 格式: R=62 G=62 B=66 → 0x423E3E
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

        let is_on = ENABLED.load(Ordering::Relaxed);
        let text: &[u16] = if is_on {
            &AUTOPLAY_ON_TEXT
        } else {
            &AUTOPLAY_WAS_USED_TEXT
        };

        DrawTextW(hdc, text.as_ptr(), text.len() as i32 - 1, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

        SelectObject(hdc, old);
        DeleteObject(font);
        EndPaint(hwnd, &ps);
        return 0;
    }
    DefWindowProcA(hwnd, msg, wp, lp)
}

static AUTOPLAY_ON_TEXT: [u16; 14] = encode_utf16_const("AutoPlay ON\0");
static AUTOPLAY_WAS_USED_TEXT: [u16; 16] = encode_utf16_const("AutoPlay \u{66FE}\u{4F7F}\u{7528}\0");

const fn encode_utf16_const<const N: usize>(s: &str) -> [u16; N] {
    let bytes = s.as_bytes();
    let mut out = [0u16; N];
    let mut i = 0;
    let mut o = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            out[o] = b as u16;
            i += 1;
        } else if b < 0xE0 {
            out[o] = ((b as u16 & 0x1F) << 6) | (bytes[i + 1] as u16 & 0x3F);
            i += 2;
        } else if b < 0xF0 {
            out[o] = ((b as u16 & 0x0F) << 12)
                | ((bytes[i + 1] as u16 & 0x3F) << 6)
                | (bytes[i + 2] as u16 & 0x3F);
            i += 3;
        } else {
            i += 4;
        }
        o += 1;
    }
    out
}

unsafe fn osd_create() {
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
        class_name: b"AppleChuAutoplayOSD\0".as_ptr(),
    };
    RegisterClassA(&wc);
    OSD_HWND = CreateWindowExA(
        WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
        b"AppleChuAutoplayOSD\0".as_ptr(),
        ptr::null(),
        WS_POPUP,
        0, 0, OSD_WIDTH, OSD_HEIGHT,
        0, 0, instance, ptr::null(),
    );
    if OSD_HWND != 0 {
        // 60% 不透明度: alpha = 0.6 * 255 = 153
        SetLayeredWindowAttributes(OSD_HWND, 0, 153, LWA_ALPHA);
    }
}

unsafe fn osd_reposition() {
    if OSD_HWND == 0 {
        return;
    }
    let screen_w = GetSystemMetrics(0);
    let x = screen_w - OSD_WIDTH - OSD_MARGIN;
    let y = OSD_MARGIN;
    SetWindowPos(OSD_HWND, 0, x, y, OSD_WIDTH, OSD_HEIGHT, 0x0014);
}

unsafe fn osd_show(visible: bool) {
    if OSD_HWND != 0 {
        ShowWindow(OSD_HWND, if visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
    }
}

fn read_u32(api: &Api, addr: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    api.mem_read(addr, &mut bytes).then(|| u32::from_le_bytes(bytes))
}

fn read_usize(api: &Api, addr: usize) -> Option<usize> {
    let mut bytes = [0; std::mem::size_of::<usize>()];
    api.mem_read(addr, &mut bytes).then(|| usize::from_le_bytes(bytes))
}

fn read_le_u32(buf: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn log_info(message: &str) {
    unsafe {
        if let Some(api) = API_HANDLE {
            api.log_info(message);
        }
    }
}
