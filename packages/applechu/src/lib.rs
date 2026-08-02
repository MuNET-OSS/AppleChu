#![allow(
    non_snake_case,
    dead_code,
    clashing_extern_declarations,
    clippy::manual_c_str_literals,
    clippy::module_inception,
    clippy::too_many_arguments
)]
#![feature(c_variadic)]

mod aime;
mod autoplay;
mod chuniio;
mod config;
mod d3d9;
mod early_patch;
mod gfx;
mod io4;
mod iohook;
mod led;
mod module_registry;
mod national_match;
mod patch_engine;
mod patches;
mod platform;
mod proxy;
mod slider;
mod system_config;
mod unlocker;
mod util;
mod ux;
mod vfd;

use std::ffi::c_char;

use crate::config::{Config, DiagnosticLevel};
use crate::util::api::{Api, ChuModAPI, ChuModInfo, API};

const NAME: &[u8] = b"AppleChu\0";
const VERSION: &[u8] = b"1.0.0\0";
const MIN_LOADER_VERSION: &[u8] = b"1.0.0\0";

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

    let Some(api_handle) = Api::new(api, info) else {
        return -1;
    };
    let _ = API.set(api_handle);

    let Some(api) = API.get() else {
        return -1;
    };

    api.log_info("--- Begin chusan_pre_startup ---");
    let config = Config::global(&base_dir(info));
    early_patch::flush_logs(api);
    for diagnostic in config.diagnostics() {
        match diagnostic.level {
            DiagnosticLevel::Warning => api.log_warn(&diagnostic.message),
            DiagnosticLevel::Error => api.log_error(&diagnostic.message),
        }
    }
    if !config.is_valid() {
        return -1;
    }
    if let Err(error) = config.sync() {
        api.log_warn(&format!("写入规范化 AppleChu.toml 失败: {error}"));
    }
    patches::install_pre_entry_hooks(api, config);

    pin_dll(api, "D3DCompiler_43.dll");
    pin_dll(api, "dbghelp.dll");

    module_registry::init_all(api, config);

    api.log_info("--- End chusan_pre_startup ---");
    0
}

fn pin_dll(api: &Api, name: &str) {
    let cname = format!("{}\0", name);
    let handle = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryA(cname.as_ptr()) };
    if !handle.is_null() {
        api.log_info(&format!("pinned {}", name));
    }
}

fn base_dir(info: *const ChuModInfo) -> String {
    let Some(info) = (unsafe { info.as_ref() }) else {
        return ".".to_owned();
    };
    if info.game_module.is_null() {
        return ".".to_owned();
    }
    let module_path = unsafe { std::ffi::CStr::from_ptr(info.game_module) }
        .to_string_lossy()
        .into_owned();
    std::path::Path::new(&module_path)
        .parent()
        .and_then(std::path::Path::to_str)
        .filter(|parent| !parent.is_empty())
        .unwrap_or(".")
        .to_owned()
}

#[no_mangle]
pub extern "C" fn chumod_shutdown() {
    if let Some(api) = API.get() {
        module_registry::shutdown_all();
        api.log_info("AppleChu unloaded");
    }
}
