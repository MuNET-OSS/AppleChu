use std::ffi::{c_char, c_void};

use crate::config::Config;
use crate::iohook::proc_addr;
use crate::platform::winapi;
use crate::util::api::Api;

pub fn init(api: &Api, config: &Config) {
    if !config.get_bool("AMVideo", "enable", true) {
        return;
    }

    proc_addr::push_get_proc_override("amvideo.dll", get_proc_override);
    proc_addr::push_load_override_a(load_override_a);
    proc_addr::push_load_override_w(load_override_w);

    api.log_info("AMVideo hook initialized");
}

pub fn shutdown() {}

fn get_proc_override(_module: usize, name: &str) -> Option<*const ()> {
    amvideo_proc(name).map(|proc| proc.cast())
}

fn load_override_a(path: &str) -> Option<usize> {
    is_amvideo_path(path).then(current_module)
}

fn load_override_w(path: &str) -> Option<usize> {
    is_amvideo_path(path).then(current_module)
}

fn is_amvideo_path(path: &str) -> bool {
    let lower = path.replace('/', "\\").to_ascii_lowercase();
    lower.ends_with("amvideo.dll") || lower == "$amvideo"
}

fn amvideo_proc(name: &str) -> Option<*const c_void> {
    match name {
        "amDllVideoOpen" => Some(am_dll_video_open as *const c_void),
        "amDllVideoClose" => Some(am_dll_video_close as *const c_void),
        "amDllVideoSetResolution" => Some(am_dll_video_set_resolution as *const c_void),
        "amDllVideoGetVBiosVersion" => Some(am_dll_video_get_vbios_version as *const c_void),
        _ => None,
    }
}

fn current_module() -> usize {
    unsafe { winapi::GetModuleHandleA(std::ptr::null()) }
}

#[no_mangle]
pub extern "C" fn am_dll_video_open(_: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn am_dll_video_close(_: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn am_dll_video_set_resolution(_: i32, _: i32, _: i32) -> i32 {
    0
}

#[no_mangle]
pub extern "C" fn am_dll_video_get_vbios_version(buffer: *mut c_char, len: u32) -> i32 {
    const VERSION: &[u8] = b"01.02.03.04.05\0";
    if buffer.is_null() || len == 0 {
        return -1;
    }

    let copy_len = VERSION.len().min(len as usize);
    unsafe {
        std::ptr::copy_nonoverlapping(VERSION.as_ptr().cast::<c_char>(), buffer, copy_len);
        *buffer.add(copy_len.saturating_sub(1)) = 0;
    }
    0
}
