#![allow(
    non_snake_case,
    dead_code,
    clashing_extern_declarations,
    clippy::manual_c_str_literals,
    clippy::missing_safety_doc,
    clippy::missing_transmute_annotations,
    clippy::module_inception,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::items_after_test_module,
    clippy::too_many_arguments
)]
mod aime;
pub mod amdaemon;
mod autoplay;
mod chuniio;
pub mod config;
mod d3d9;
#[cfg(target_arch = "x86")]
mod early_patch;
mod gfx;
mod io4;
pub mod iohook;
mod led;
pub mod module_registry;
#[cfg(target_arch = "x86")]
mod national_match;
mod patch_engine;
mod patches;
pub mod platform;
#[cfg(target_arch = "x86")]
mod proxy;
#[cfg(target_arch = "x86")]
mod schema_embed;
mod slider;
mod system_config;
mod unlocker;
pub mod util;
mod ux;
mod vfd;

#[cfg(target_arch = "x86")]
use std::ffi::c_char;

#[cfg(target_arch = "x86")]
use crate::config::{Config, DiagnosticLevel};
use crate::util::api::Api;
#[cfg(target_arch = "x86")]
use crate::util::api::API;
#[cfg(target_arch = "x86")]
use crate::util::api::{ChuModAPI, ChuModInfo};

#[cfg(target_arch = "x86")]
const NAME: &[u8] = b"AppleChu\0";
#[cfg(target_arch = "x86")]
const VERSION: &[u8] = b"2.0.0\0";
#[cfg(target_arch = "x86")]
const MIN_LOADER_VERSION: &[u8] = b"1.0.0\0";

#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn chumod_name() -> *const c_char {
    NAME.as_ptr().cast()
}

#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn chumod_version() -> *const c_char {
    VERSION.as_ptr().cast()
}

#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn chumod_min_loader_version() -> *const c_char {
    MIN_LOADER_VERSION.as_ptr().cast()
}

#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn chumod_init(info: *const ChuModInfo, api: *const ChuModAPI) -> i32 {
    if info.is_null() || api.is_null() {
        return -1;
    }

    // SAFETY: loader 在 chumod_init 调用期间提供有效 ABI 指针，API 表在模块存活期内保持有效
    let Some(api_handle) = (unsafe { Api::new(api, info) }) else {
        return -1;
    };
    let _ = API.set(api_handle);

    let Some(api) = API.get() else {
        return -1;
    };

    api.log_info("Game feature initialization started");
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
    patches::install_pre_entry_hooks(api, config);

    pin_dll(api, "D3DCompiler_43.dll");
    pin_dll(api, "dbghelp.dll");

    module_registry::init_all(api, config);

    api.log_info("Game feature initialization completed");
    0
}

fn pin_dll(api: &Api, name: &str) {
    let cname = format!("{}\0", name);
    let handle = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryA(cname.as_ptr()) };
    if handle.is_null() {
        api.log_warn(&format!("Failed to load dependency: {name}"));
    } else {
        api.log_info(&format!("Dependency loaded: {name}"));
    }
}

#[cfg(target_arch = "x86")]
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

#[cfg(target_arch = "x86")]
#[no_mangle]
pub extern "C" fn chumod_shutdown() {
    if let Some(api) = API.get() {
        module_registry::shutdown_all();
        api.log_info("AppleChu game module unloaded");
    }
}
