mod hook;
mod recovery;
mod fps_osd;

use std::ffi::c_void;

use crate::config::Config;
use crate::util::api::Api;

#[repr(C)]
struct D3D9ProxyAPI {
    set_frame_lock: unsafe extern "C" fn(u32),
    get_device: unsafe extern "C" fn() -> *mut c_void,
    get_hwnd: unsafe extern "C" fn() -> usize,
    register_present_callback: unsafe extern "C" fn(unsafe extern "C" fn(*mut c_void)),
}

type GetAPIFn = unsafe extern "C" fn() -> *const D3D9ProxyAPI;

pub fn init_all(api: &Api, config: &Config) {
    if config.is_enabled("DeviceLostFix") {
        hook::init(api);
    }
    init_d3d9_proxy(api, config);
}

fn init_d3d9_proxy(api: &Api, config: &Config) {
    let proxy = unsafe { get_proxy_api() };
    let Some(proxy) = proxy else {
        return;
    };

    if config.is_enabled("FpsDisplay") {
        unsafe { ((*proxy).register_present_callback)(fps_osd::on_end_scene) };
        api.log_info("FPS display registered (d3d9 proxy callback)");
    }

    if config.is_enabled("FrameLock") {
        let fps = config.get_int("FrameLock", "fps", 0) as u32;
        if fps > 0 {
            unsafe { ((*proxy).set_frame_lock)(fps) };
            api.log_info(&format!("frame lock: {}fps (d3d9 proxy)", fps));
        }
    }
}

unsafe fn get_proxy_api() -> Option<*const D3D9ProxyAPI> {
    let d3d9 = GetModuleHandleA(b"d3d9.dll\0".as_ptr());
    if d3d9 == 0 {
        return None;
    }
    let proc = GetProcAddress(d3d9, b"d3d9proxy_get_api\0".as_ptr());
    if proc.is_null() {
        return None;
    }
    let proc = proc as *const ();
    let get_api: GetAPIFn = std::mem::transmute(proc);
    let api = get_api();
    (!api.is_null()).then_some(api)
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(name: *const u8) -> usize;
    fn GetProcAddress(module: usize, name: *const u8) -> *const c_void;
}
