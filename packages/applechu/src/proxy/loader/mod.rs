pub mod console;
pub mod crash_dump;
pub mod crash_ui;
pub mod crash_zip;
pub mod dependency;
mod external;
pub mod frame_hook;
pub mod hash;
pub mod hot_reload;
pub mod log;
pub mod metadata;
pub mod pe;
pub mod scanner;
pub mod seh;
pub mod state;

use std::ffi::c_char;
use std::fs::File;

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleA;

use chu_abi::{ChuModInfo, CHUMOD_API_VERSION};

use crate::config::Config;
use crate::proxy::api_impl;

use self::log::{log_info, write_log_inner};
use self::pe::{get_self_base_dir, parse_game_info};
use self::seh::{call_mod_on_ready, call_mod_shutdown};
use self::state::STATE;

extern "system" {
    fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
}

pub unsafe fn load_mods() {
    let mut state = STATE.lock().unwrap();
    if state.loaded {
        return;
    }
    state.loaded = true;

    let base_dir = match get_self_base_dir() {
        Some(d) => d,
        None => return,
    };
    let config = Config::global(&base_dir);
    let enable_console = config
        .section::<crate::system_config::SystemConfig>()
        .is_none_or(|config| config.enable_console);
    console::init(&mut state, enable_console);

    state.base_dir = base_dir.clone();
    state.log_file = File::create(format!("{}\\chumod_loader.log", base_dir)).ok();
    write_log_inner(&mut state, &format!("loader start: base={}", base_dir));
    drop(state);

    api_impl::init();

    let game = GetModuleHandleA(b"chusanApp.exe\0".as_ptr());
    let (game_size, text_base, text_size, rdata_base, rdata_size) = if !game.is_null() {
        parse_game_info(game)
    } else {
        (0, 0, 0, 0, 0)
    };
    api_impl::set_rtti_info(rdata_base, rdata_size as usize, text_base);

    let game_module_str: *const c_char = if !game.is_null() {
        b"chusanApp.exe\0".as_ptr().cast()
    } else {
        std::ptr::null()
    };
    let loader_ver = concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast();
    let info = ChuModInfo {
        api_version: CHUMOD_API_VERSION,
        loader_version: loader_ver,
        game_module: game_module_str,
        game_base: if !game.is_null() { game as usize } else { 0 },
        game_size,
        text_base,
        text_size,
        rdata_base,
        rdata_size,
    };
    let api = api_impl::get_api();
    let builtin_loaded = crate::chumod_init(&info, api) == 0;
    if let Ok(mut state) = STATE.lock() {
        state.builtin_loaded = builtin_loaded;
    }
    if builtin_loaded {
        log_info("loaded builtin mod: AppleChu");
    } else {
        log_info("builtin mod init failed: AppleChu");
    }

    external::load(&base_dir, &info, api);

    let mut state = STATE.lock().unwrap();
    let count = state.mods.len();
    write_log_inner(&mut state, &format!("mods loaded: {}", count));
    let ready_mods: Vec<_> = state
        .mods
        .iter()
        .filter_map(|m| m.on_ready.map(|on_ready| (m.name.clone(), on_ready)))
        .collect();
    drop(state);

    for (name, on_ready) in ready_mods {
        call_mod_on_ready(&name, on_ready);
    }

    frame_hook::start_if_needed();
    hot_reload::start_monitor();
}

pub unsafe fn unload_mods() {
    hot_reload::stop_monitor();
    frame_hook::stop();
    let mut state = STATE.lock().unwrap();
    while let Some(m) = state.mods.pop() {
        write_log_inner(&mut state, &format!("unloading mod: {}", m.name));
        if let Some(shutdown) = m.shutdown {
            call_mod_shutdown(&m.name, shutdown);
        }
        if !m.handle.is_null() {
            FreeLibrary(m.handle);
        }
    }
    state.loaded = false;
    let builtin_loaded = state.builtin_loaded;
    state.builtin_loaded = false;
    drop(state);

    if builtin_loaded {
        crate::chumod_shutdown();
    }

    api_impl::shutdown();

    let mut state = STATE.lock().unwrap();
    write_log_inner(&mut state, "loader shutdown");
    state.log_file = None;
}
