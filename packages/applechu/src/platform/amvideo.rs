use std::ffi::{c_char, c_void};
use std::ptr;

use crate::iohook::proc_addr;
use crate::platform::reg_hook::{self, RegValue, HKEY_LOCAL_MACHINE};
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct AmVideoConfig => AM_VIDEO_CONFIG_SECTION {
        section: "AMVideo",
        order: 920,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "AMVideo 平台模拟",
        fields: {}
    }
}

#[applechu_macros::config_section(stage = Platform, order = 10)]
pub(crate) fn init(api: &Api, _config: &AmVideoConfig) {
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty\\amVideo",
        vec![
            RegValue::string("name", "$amvideo"),
            RegValue::string("name_x86", "$amvideo"),
        ],
    );
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty\\sgsetdisplaysetting\\CurrentSetting",
        vec![
            RegValue::string("monitor_setting_1", "0"),
            RegValue::string("monitor_setting_2", "0"),
            RegValue::dword("port_1", 1),
            RegValue::dword("port_2", 1),
            RegValue::dword("port_3", 1),
            RegValue::dword("port_4", 1),
            RegValue::dword("port_5", 1),
            RegValue::dword("port_6", 1),
            RegValue::dword("port_7", 1),
            RegValue::dword("port_8", 1),
            RegValue::string("resolution_1", "1920x1080"),
            RegValue::string("resolution_2", "1920x1080"),
            RegValue::dword("use_segatiming", 0),
        ],
    );
    proc_addr::push_get_proc_override("amvideo.dll", get_proc_override);
    proc_addr::push_load_override_a(load_override_a);
    proc_addr::push_load_override_w(load_override_w);

    api.log_info("AMVideo emulator ready");
}

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
    // GetModuleHandle(NULL) 返回的是宿主 EXE；伪 DLL 必须返回包含本函数的代理模块
    let mut module = ptr::null_mut();
    let flags = windows_sys::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
        | windows_sys::Win32::System::LibraryLoader::GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    let ok = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleHandleExW(
            flags,
            am_dll_video_open as *const u16,
            &mut module,
        )
    };
    if ok == 0 {
        0
    } else {
        module as usize
    }
}

#[cfg_attr(target_arch = "x86", no_mangle)]
pub extern "C" fn am_dll_video_open(_: *mut c_void) -> i32 {
    log_call("AMVideo opened");
    0
}

#[cfg_attr(target_arch = "x86", no_mangle)]
pub extern "C" fn am_dll_video_close(_: *mut c_void) -> i32 {
    log_call("AMVideo closed");
    0
}

#[cfg_attr(target_arch = "x86", no_mangle)]
pub extern "C" fn am_dll_video_set_resolution(_: *mut c_void, _: *mut c_void) -> i32 {
    log_call("AMVideo resolution configured");
    0
}

#[cfg_attr(target_arch = "x86", no_mangle)]
pub unsafe extern "C" fn am_dll_video_get_vbios_version(
    _: *mut c_void,
    buffer: *mut c_char,
    len: usize,
) -> i32 {
    log_call("AMVideo firmware version requested");
    const VERSION: &[u8] = b"01.02.03.04.05\0";
    if buffer.is_null() || len == 0 {
        return -1;
    }

    let copy_len = VERSION.len().min(len);
    std::ptr::copy_nonoverlapping(VERSION.as_ptr().cast::<c_char>(), buffer, copy_len);
    *buffer.add(copy_len.saturating_sub(1)) = 0;
    0
}

fn log_call(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}
