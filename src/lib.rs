#![allow(non_snake_case, clippy::manual_c_str_literals)]

mod config;
mod d3d9;
mod hooks;
mod patch_engine;
mod patches;
mod util;
mod ux;

use std::ffi::{c_char, CStr};

use crate::config::Config;
use crate::util::api::{Api, ChuModAPI, ChuModInfo, API};

const NAME: &[u8] = b"AppleChu\0";
const VERSION: &[u8] = b"1.0.0\0";
const MIN_LOADER_VERSION: &[u8] = b"3.0.0\0";

#[no_mangle]
pub extern "C" fn chumod_name() -> *const c_char {
    NAME.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn chumod_version() -> *const c_char {
    VERSION.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn chumod_min_loader_version() -> *const c_char {
    MIN_LOADER_VERSION.as_ptr().cast()
}

#[no_mangle]
pub extern "C" fn chumod_init(info: *const ChuModInfo, api: *const ChuModAPI) -> i32 {
    if info.is_null() || api.is_null() {
        return -1;
    }

    let api_handle = Api::new(api, info);
    let _ = API.set(api_handle);

    let Some(api) = API.get() else {
        return -1;
    };

    api.log_info("AppleChu 初始化中");
    let config = Config::load(&base_dir(info));
    patches::apply_all(api, &config);
    hooks::init_all(api, &config);
    ux::init_all(api, &config);
    d3d9::init_all(api, &config);
    api.log_info("AppleChu 初始化完成");
    0
}

fn base_dir(info: *const ChuModInfo) -> String {
    let Some(info) = (unsafe { info.as_ref() }) else {
        return ".".to_owned();
    };

    if info.game_module.is_null() {
        return ".".to_owned();
    }

    let module_path = unsafe { CStr::from_ptr(info.game_module) }
        .to_string_lossy()
        .into_owned();

    std::path::Path::new(&module_path)
        .parent()
        .and_then(std::path::Path::to_str)
        .unwrap_or(".")
        .to_owned()
}

#[no_mangle]
pub extern "C" fn chumod_shutdown() {
    if let Some(api) = API.get() {
        hooks::shutdown_all();
        api.log_info("AppleChu 已卸载");
    }
}
