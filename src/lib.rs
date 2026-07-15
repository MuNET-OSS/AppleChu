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
mod national_match;
mod patch_engine;
mod patches;
mod platform;
mod proxy;
mod slider;
mod util;
mod ux;
mod vfd;

use std::ffi::c_char;

use crate::config::Config;
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
    let config = Config::load(&base_dir(info));
    early_patch::flush_logs(api);
    patches::apply_pre_entry(api, &config);
    patches::install_pre_entry_hooks(api, &config);

    pin_dll(api, "D3DCompiler_43.dll");
    pin_dll(api, "dbghelp.dll");

    gfx::init_all(api, &config);

    let enable_platform = config.get_bool("HookMode", "platform", true);
    let enable_devices = config.get_bool("HookMode", "devices", true);

    if enable_platform {
        iohook::proc_addr::init(api);
        if config.get_bool("HookMode", "platformModules", true) {
            platform::init_all(api, &config);
        } else {
            api.log_info("platform modules DISABLED (diag)");
        }
        if config.get_bool("HookMode", "iohook", true) {
            iohook::init_all(api, &config);
        } else {
            api.log_info("iohook DISABLED (diag)");
        }
    }

    if enable_devices {
        let is_sp = config.is_sp_mode();

        chuniio::init(api, &config);
        io4::init(api, &config);
        slider::init(api, &config);

        if is_sp {
            vfd::init(api, &config);
        }

        let default_led: [u32; 2] = if is_sp { [20, 21] } else { [2, 3] };
        let led_ports: [u32; 2] = [
            config.get_int_alias(
                &[
                    ("Led15093", "port0"),
                    ("Led15093", "portNo1"),
                    ("led15093", "portNo1"),
                ],
                default_led[0] as i64,
            ) as u32,
            config.get_int_alias(
                &[
                    ("Led15093", "port1"),
                    ("Led15093", "portNo2"),
                    ("led15093", "portNo2"),
                ],
                default_led[1] as i64,
            ) as u32,
        ];
        led::init(api, &config, &led_ports);

        let default_aime: u32 = if is_sp { 3 } else { 2 };
        let aime_port = config.get_int_alias(
            &[
                ("Aime", "port"),
                ("Aime", "port_no"),
                ("Aime", "portNo"),
                ("aime", "portNo"),
            ],
            default_aime as i64,
        ) as u32;
        aime::init(api, &config, aime_port);
    }

    if !enable_platform {
        api.log_info("platform hooks DISABLED");
    }
    if !enable_devices {
        api.log_info("device emulation DISABLED");
    }

    national_match::init(api, &config);
    autoplay::init_all(api, &config);
    ux::init_all(api, &config);
    d3d9::init_all(api, &config);

    api.log_info("--- End chusan_pre_startup ---");
    0
}

fn pin_dll(api: &Api, name: &str) {
    let cname = format!("{}\0", name);
    let handle = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryA(cname.as_ptr()) };
    if handle != 0 {
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
        autoplay::shutdown_all();
        platform::shutdown_all();
        api.log_info("AppleChu unloaded");
    }
}
