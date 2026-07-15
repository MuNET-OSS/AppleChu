use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::util::api::Api;

static ENABLED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static WAS_USED: AtomicBool = AtomicBool::new(false);
static HOTKEY: AtomicI32 = AtomicI32::new(VK_HOME);

static mut API_HANDLE: Option<Api> = None;
static mut STATE_PTR: usize = 0;
static mut DEMO_OFF: u32 = 0;
static mut PRESET_OFF: u32 = 0;
static mut JUDGE_ADDR: usize = 0;
static mut ORIG_JUDGE: usize = 0;
static mut UPSERT_ADDR: usize = 0;
static mut ORIG_UPSERT: usize = 0;

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
}

#[link(name = "user32")]
extern "system" {
    pub fn GetAsyncKeyState(vkey: i32) -> i16;
}
const VK_HOME: i32 = 0x24;

pub fn init(api: &Api, hotkey: &str) {
    unsafe { API_HANDLE = Some(*api) };
    let hotkey = parse_hotkey(hotkey).unwrap_or(VK_HOME);
    HOTKEY.store(hotkey, Ordering::Relaxed);
    RUNNING.store(true, Ordering::Relaxed);

    let text_base = api.text_base();
    let text_size = api.text_size();
    let game_base = api.game_base();
    let game_size = api.game_size();
    if text_base == 0 || text_size == 0 || game_base == 0 || game_size == 0 {
        api.log_error("Autoplay init failed: invalid game module info");
        return;
    }

    let demo_checker = find_demo_checker(api, text_base, text_size);
    if demo_checker == 0 {
        api.log_error("Autoplay init failed: demo checker not found");
        return;
    }
    let Some(demo_off) = read_u32(api, demo_checker + 2) else {
        api.log_error("Autoplay init failed: failed to read demo offset");
        return;
    };
    let Some(preset_off) = read_u32(api, demo_checker + 11) else {
        api.log_error("Autoplay init failed: failed to read preset offset");
        return;
    };
    unsafe {
        DEMO_OFF = demo_off;
        PRESET_OFF = preset_off;
    }
    api.log_info(&format!(
        "Autoplay demo checker @ 0x{demo_checker:08X}, offsets +0x{demo_off:X}/+0x{preset_off:X}"
    ));

    let judge = find_judge_check(api, text_base, text_size);
    if judge == 0 {
        api.log_error("Autoplay init failed: JudgeTapChecker::check not found");
        return;
    }
    unsafe { JUDGE_ADDR = judge };
    api.log_info(&format!("Autoplay JudgeTapChecker::check @ 0x{judge:08X}"));

    let state_ptr = find_state_ptr(
        api,
        text_base,
        text_size,
        demo_checker,
        game_base,
        game_size,
    );
    if state_ptr == 0 {
        api.log_error("Autoplay init failed: global game state pointer not found");
        return;
    }
    unsafe { STATE_PTR = state_ptr };
    api.log_info(&format!("Autoplay game state ptr @ 0x{state_ptr:08X}"));

    let Some(trampoline) = api.hook_create(judge, hooked_judge_check as *const () as usize) else {
        api.log_error("Autoplay init failed: failed to create judge hook");
        return;
    };
    unsafe { ORIG_JUDGE = trampoline };
    if !api.hook_enable(judge) {
        api.log_error("Autoplay init failed: failed to enable judge hook");
        return;
    }

    unsafe {
        CreateThread(
            ptr::null(),
            0,
            Some(demo_write_thread),
            ptr::null(),
            0,
            ptr::null_mut(),
        );
        CreateThread(
            ptr::null(),
            0,
            Some(hotkey_thread),
            ptr::null(),
            0,
            ptr::null_mut(),
        );
    }
    api.log_info(&format!(
        "Autoplay enabled: toggle with {}, writes demo/preset flag when on",
        hotkey_name(hotkey)
    ));
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

fn reset_was_used() {
    WAS_USED.store(false, Ordering::Relaxed);
}

fn find_demo_checker(api: &Api, text_base: usize, text_size: u32) -> usize {
    let sig = [
        0x80, 0xB9, 0, 0, 0, 0, 0x00, 0x74, 0x00, 0x83, 0xB9, 0, 0, 0, 0, 0x00, 0x0F, 0x95, 0xC0,
        0xC3, 0x32, 0xC0, 0xC3,
    ];
    api.aob_scan(text_base, text_size, &sig, "xx????xx?xx????xxxxxxxx")
}

fn find_judge_check(api: &Api, text_base: usize, text_size: u32) -> usize {
    let sig: [u8; 22] = [
        0xF3, 0x0F, 0x10, 0x41, 0x04, 0xF3, 0x0F, 0x10, 0x54, 0x24, 0x04, 0x0F, 0x2F, 0xC2, 0x76,
        0x00, 0x83, 0xC8, 0xFF, 0xC2, 0x08, 0x00,
    ];
    let found = api.aob_scan(text_base, text_size, &sig, "xxxxxxxxxxxxxxx?xxxxxx");
    if found == 0 {
        return 0;
    }
    for back in 1..0x200usize {
        if found < back + text_base {
            break;
        }
        let addr = found - back;
        let mut buf = [0u8; 5];
        if !api.mem_read(addr, &mut buf) {
            continue;
        }
        if buf[0] == 0x55 && buf[1] == 0x8B && buf[2] == 0xEC && buf[3] == 0x6A && buf[4] == 0xFF {
            return addr;
        }
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
    while RUNNING.load(Ordering::Relaxed) {
        if GetAsyncKeyState(HOTKEY.load(Ordering::Relaxed)) & 1 != 0 {
            let new_state = !ENABLED.load(Ordering::Relaxed);
            ENABLED.store(new_state, Ordering::Relaxed);
            if new_state {
                WAS_USED.store(true, Ordering::Relaxed);
            }
        }
        Sleep(50);
    }
    0
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

pub fn init_upload_guard(api: &Api) {
    unsafe {
        API_HANDLE = Some(*api);
    }

    let upsert = find_upsert_function(
        api,
        api.text_base(),
        api.text_size(),
        api.game_base(),
        api.game_size(),
    );
    if upsert == 0 {
        api.log_warn("Autoplay upload guard init failed: UpsertUserAll not found, scores may upload during autoplay");
        return;
    }

    let Some(trampoline) = api.hook_create(upsert, hooked_upsert as *const () as usize) else {
        api.log_warn("Autoplay upload guard init failed: failed to create UpsertUserAll hook");
        return;
    };
    if !api.hook_enable(upsert) {
        api.log_warn("Autoplay upload guard init failed: failed to enable UpsertUserAll hook");
        return;
    }

    unsafe {
        UPSERT_ADDR = upsert;
        ORIG_UPSERT = trampoline;
    }
    api.log_info(&format!(
        "Autoplay upload guard enabled: UpsertUserAll @ 0x{upsert:08X}, scores blocked during autoplay"
    ));
}

pub fn shutdown_upload_guard() {
    unsafe {
        if let Some(api) = API_HANDLE {
            if UPSERT_ADDR != 0 {
                api.hook_disable(UPSERT_ADDR);
                api.hook_remove(UPSERT_ADDR);
                UPSERT_ADDR = 0;
            }
            api.log_info("Autoplay upload guard cleaned up");
        }
    }
}

unsafe extern "C" fn hooked_upsert(a: *mut c_void, b: *mut c_void, c: *mut c_void) {
    if is_enabled() || was_used() {
        if let Some(api) = API_HANDLE {
            api.log_info("Autoplay upload guard: upload blocked because autoplay was used");
        }
        reset_was_used();
        return;
    }

    let orig: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(ORIG_UPSERT);
    orig(a, b, c);
}

fn find_upsert_function(
    api: &Api,
    text_base: usize,
    text_size: u32,
    module_base: usize,
    module_size: u32,
) -> usize {
    let str_addr = api.aob_scan(
        module_base,
        module_size,
        b"upsertUserAll\0",
        "xxxxxxxxxxxxxx",
    );
    if str_addr == 0 {
        return 0;
    }

    let mut push_pattern = [0u8; 5];
    push_pattern[0] = 0x68;
    push_pattern[1..5].copy_from_slice(&(str_addr as u32).to_le_bytes());
    let push_site = api.aob_scan(text_base, text_size, &push_pattern, "xxxxx");
    if push_site == 0 {
        return 0;
    }

    let mut text = vec![0; text_size as usize];
    if !api.mem_read(text_base, &mut text) {
        return 0;
    }
    let Some(push_offset) = push_site.checked_sub(text_base) else {
        return 0;
    };

    for back in 1..0x200usize {
        let Some(q) = push_offset.checked_sub(back) else {
            break;
        };
        if q + 5 > text.len() {
            continue;
        }
        if text[q..q + 5] == [0x55, 0x8B, 0xEC, 0x6A, 0xFF] {
            let data_builder = text_base + q;
            return find_caller_function(&text, text_base, data_builder).unwrap_or(0);
        }
    }
    0
}

fn find_caller_function(text: &[u8], text_base: usize, data_builder: usize) -> Option<usize> {
    for j in 0..text.len().saturating_sub(5) {
        if text[j] != 0xE8 {
            continue;
        }
        let rel = read_le_i32(text, j + 1)?;
        let call_target = (text_base + j + 5).wrapping_add(rel as usize);
        let matched = call_target == data_builder
            || jump_target(text, text_base, call_target) == Some(data_builder);
        if !matched {
            continue;
        }

        for back in 1..0x200usize {
            let Some(f) = j.checked_sub(back) else {
                break;
            };
            if f + 5 <= text.len() && text[f..f + 5] == [0x55, 0x8B, 0xEC, 0x6A, 0xFF] {
                return Some(text_base + f);
            }
        }
    }
    None
}

fn jump_target(text: &[u8], text_base: usize, target: usize) -> Option<usize> {
    let offset = target.checked_sub(text_base)?;
    if offset + 5 > text.len() || text[offset] != 0xE9 {
        return None;
    }
    let rel = read_le_i32(text, offset + 1)?;
    Some((target + 5).wrapping_add(rel as usize))
}

fn parse_hotkey(value: &str) -> Option<i32> {
    let key = value.trim();
    if key.is_empty() {
        return None;
    }
    if let Some(hex) = key.strip_prefix("0x").or_else(|| key.strip_prefix("0X")) {
        return i32::from_str_radix(hex, 16).ok().filter(|value| *value > 0);
    }
    if let Ok(value) = key.parse::<i32>() {
        return (value > 0).then_some(value);
    }

    let normalized = key
        .trim_start_matches("VK_")
        .replace([' ', '-', '_'], "")
        .to_ascii_uppercase();
    match normalized.as_str() {
        "BACKSPACE" | "BACK" => Some(0x08),
        "TAB" => Some(0x09),
        "ENTER" | "RETURN" => Some(0x0D),
        "SHIFT" => Some(0x10),
        "CTRL" | "CONTROL" => Some(0x11),
        "ALT" | "MENU" => Some(0x12),
        "PAUSE" => Some(0x13),
        "CAPSLOCK" | "CAPITAL" => Some(0x14),
        "ESC" | "ESCAPE" => Some(0x1B),
        "SPACE" => Some(0x20),
        "PAGEUP" | "PRIOR" => Some(0x21),
        "PAGEDOWN" | "NEXT" => Some(0x22),
        "END" => Some(0x23),
        "HOME" => Some(VK_HOME),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        "INSERT" => Some(0x2D),
        "DELETE" | "DEL" => Some(0x2E),
        key if key.len() == 1 => key.as_bytes().first().copied().map(i32::from),
        key if key.starts_with('F') => key[1..]
            .parse::<i32>()
            .ok()
            .filter(|value| (1..=24).contains(value))
            .map(|value| 0x70 + value - 1),
        _ => None,
    }
}

fn hotkey_name(vk: i32) -> String {
    match vk {
        VK_HOME => "Home".to_owned(),
        0x21 => "PageUp".to_owned(),
        0x22 => "PageDown".to_owned(),
        0x23 => "End".to_owned(),
        0x2D => "Insert".to_owned(),
        0x2E => "Delete".to_owned(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk as u32).unwrap_or('?').to_string(),
        _ => format!("VK 0x{vk:02X}"),
    }
}

fn read_u32(api: &Api, addr: usize) -> Option<u32> {
    let mut bytes = [0; 4];
    api.mem_read(addr, &mut bytes)
        .then(|| u32::from_le_bytes(bytes))
}

fn read_usize(api: &Api, addr: usize) -> Option<usize> {
    let mut bytes = [0; std::mem::size_of::<usize>()];
    api.mem_read(addr, &mut bytes)
        .then(|| usize::from_le_bytes(bytes))
}

fn read_le_u32(buf: &[u8], offset: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(offset..offset + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_le_i32(buf: &[u8], offset: usize) -> Option<i32> {
    let bytes: [u8; 4] = buf.get(offset..offset + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(bytes))
}

fn log_info(message: &str) {
    unsafe {
        if let Some(api) = API_HANDLE {
            api.log_info(message);
        }
    }
}
