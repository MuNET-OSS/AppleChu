mod monitor;

pub use monitor::{start_monitor, stop_monitor};

use std::ffi::{c_char, CStr, CString};
use std::fs::File;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleHandleA, GetProcAddress, LoadLibraryA,
};

use chu_abi::{
    ChuModFrameFunc, ChuModInfo, ChuModInitFunc, ChuModNameFunc, ChuModReadyFunc,
    ChuModShutdownFunc, CHUMOD_API_VERSION,
};

use crate::proxy::api_impl;

use super::frame_hook;
use super::log::{log_error, log_info, log_warn};
use super::pe::parse_game_info;
use super::seh::{call_mod_init, call_mod_on_ready, call_mod_shutdown};
use super::state::{LoadedMod, STATE};

static RELOAD_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

extern "system" {
    fn CreateDirectoryA(path: *const u8, security: *const std::ffi::c_void) -> i32;
    fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
}

pub fn is_reloading() -> bool {
    RELOAD_IN_PROGRESS.load(Ordering::SeqCst)
}

pub unsafe extern "C" fn api_reload_mod(mod_name: *const c_char) -> i32 {
    if mod_name.is_null() {
        return -1;
    }
    let name = CStr::from_ptr(mod_name).to_string_lossy().into_owned();
    match catch_unwind(AssertUnwindSafe(|| unsafe { reload_mod_by_name(&name) })) {
        Ok(ret) => ret,
        Err(_) => {
            log_error(&format!("reload_mod panic caught: {}", name));
            -1
        }
    }
}

pub unsafe fn reload_all_mods() -> i32 {
    let names: Vec<String> = STATE
        .lock()
        .map(|state| state.mods.iter().map(|m| m.name.clone()).collect())
        .unwrap_or_default();
    let mut failed = 0;
    for name in names {
        if reload_mod_by_name(&name) != 0 {
            failed += 1;
        }
    }
    if failed == 0 {
        0
    } else {
        -1
    }
}

pub unsafe fn reload_mod_by_name(name: &str) -> i32 {
    if RELOAD_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        log_warn(&format!("reload already in progress, skip: {}", name));
        return -1;
    }
    let result = reload_mod_by_name_inner(name);
    RELOAD_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

unsafe fn reload_mod_by_name_inner(name: &str) -> i32 {
    let (index, old_mod) = {
        let mut state = match STATE.lock() {
            Ok(state) => state,
            Err(_) => return -1,
        };
        let wanted = normalize_name(name);
        let Some(index) = state.mods.iter().position(|m| mod_matches(m, &wanted)) else {
            log_warn(&format!("reload target not found: {}", name));
            return -1;
        };
        let old_mod = state.mods.remove(index);
        (index, old_mod)
    };

    log_info(&format!("reload start: {}", old_mod.name));
    if let Some(shutdown) = old_mod.shutdown {
        log_info(&format!("reload shutdown: {}", old_mod.name));
        call_mod_shutdown(&old_mod.name, shutdown);
    }
    if !old_mod.handle.is_null() {
        log_info(&format!("reload FreeLibrary: {}", old_mod.name));
        FreeLibrary(old_mod.handle);
    }

    match load_replacement(&old_mod.file_name, &old_mod.full_path) {
        Some(new_mod) => {
            let ready = new_mod
                .on_ready
                .map(|on_ready| (new_mod.name.clone(), on_ready));
            let new_name = new_mod.name.clone();
            if let Ok(mut state) = STATE.lock() {
                let insert_at = index.min(state.mods.len());
                state.mods.insert(insert_at, new_mod);
            }
            if let Some((ready_name, on_ready)) = ready {
                call_mod_on_ready(&ready_name, on_ready);
            }
            frame_hook::start_if_needed();
            log_info(&format!("reload complete: {}", new_name));
            0
        }
        None => {
            log_error(&format!(
                "reload failed, mod remains unloaded: {}",
                old_mod.name
            ));
            -1
        }
    }
}

unsafe fn load_replacement(file_name: &str, full_path: &str) -> Option<LoadedMod> {
    log_info(&format!("reload LoadLibrary: {}", full_path));
    let full_path_c = CString::new(full_path).ok()?;
    let mod_handle = LoadLibraryA(full_path_c.as_ptr() as *const u8);
    if mod_handle.is_null() {
        log_error(&format!(
            "reload LoadLibrary failed: {} (err={})",
            full_path,
            GetLastError()
        ));
        return None;
    }

    let display_name = read_display_name(mod_handle).unwrap_or_else(|| file_name.to_string());
    let init_fn_ptr = GetProcAddress(mod_handle, b"chumod_init\0".as_ptr());
    if let Some(init_fn) = init_fn_ptr {
        let init_fn: ChuModInitFunc = std::mem::transmute(init_fn);
        if call_reloaded_init(file_name, &display_name, init_fn) != Some(0) {
            FreeLibrary(mod_handle);
            return None;
        }
    }

    let shutdown_ptr = GetProcAddress(mod_handle, b"chumod_shutdown\0".as_ptr());
    let shutdown: Option<ChuModShutdownFunc> = shutdown_ptr.map(|f| std::mem::transmute(f));
    let on_ready_ptr = GetProcAddress(mod_handle, b"chumod_on_ready\0".as_ptr());
    let on_ready: Option<ChuModReadyFunc> = on_ready_ptr.map(|f| std::mem::transmute(f));
    let on_frame_ptr = GetProcAddress(mod_handle, b"chumod_on_frame\0".as_ptr());
    let on_frame: Option<ChuModFrameFunc> = on_frame_ptr.map(|f| std::mem::transmute(f));

    Some(LoadedMod {
        handle: mod_handle,
        on_ready,
        on_frame,
        shutdown,
        file_name: file_name.to_string(),
        full_path: full_path.to_string(),
        name: display_name,
    })
}

unsafe fn call_reloaded_init(
    file_name: &str,
    display_name: &str,
    init_fn: ChuModInitFunc,
) -> Option<i32> {
    let base_dir = STATE
        .lock()
        .map(|state| state.base_dir.clone())
        .unwrap_or_default();
    let game = GetModuleHandleA(b"chusanApp.exe\0".as_ptr());
    let (game_size, text_base, text_size, rdata_base, rdata_size) = if !game.is_null() {
        parse_game_info(game)
    } else {
        (0, 0, 0, 0, 0)
    };
    let loader_ver = concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *const c_char;
    let info = ChuModInfo {
        api_version: CHUMOD_API_VERSION,
        loader_version: loader_ver,
        game_module: if !game.is_null() {
            b"chusanApp.exe\0".as_ptr() as *const c_char
        } else {
            std::ptr::null()
        },
        game_base: if !game.is_null() { game as usize } else { 0 },
        game_size,
        text_base,
        text_size,
        rdata_base,
        rdata_size,
    };

    let mod_stem = file_stem(file_name);
    let config_dir = format!("{}\\mods\\config", base_dir);
    CreateDirectoryA(format!("{}\0", config_dir).as_ptr(), std::ptr::null());
    let log_dir = format!("{}\\mods\\log", base_dir);
    CreateDirectoryA(format!("{}\0", log_dir).as_ptr(), std::ptr::null());
    let mod_log_path = format!("{}\\{}.log", log_dir, mod_stem);
    let mod_log_path_c = CString::new(mod_log_path.clone()).unwrap_or_default();
    let toml_config_path = format!("{}\\{}.toml", config_dir, mod_stem);
    let ini_config_path = format!("{}\\{}.ini", config_dir, mod_stem);
    let toml_config_exists = Path::new(&toml_config_path).exists();
    let manifest_path = format!("{}\\mods\\manifest\\{}.toml", base_dir, mod_stem);
    let manifest_exists = Path::new(&manifest_path).exists();

    let api = api_impl::get_api();
    api_impl::set_log_path(mod_log_path_c.as_ptr());
    api_impl::set_current_config(if toml_config_exists {
        &toml_config_path
    } else {
        &ini_config_path
    });
    api_impl::load_current_toml_config(toml_config_exists.then_some(toml_config_path.as_str()));
    api_impl::set_current_manifest_path(manifest_exists.then_some(manifest_path.as_str()));

    if let Ok(mut state) = STATE.lock() {
        state.current_mod_log_file = File::create(&mod_log_path).ok();
    }
    let ret = call_mod_init(display_name, init_fn, &info, api);
    if let Ok(mut state) = STATE.lock() {
        state.current_mod_log_file = None;
    }
    api_impl::set_log_path(std::ptr::null());
    api_impl::load_current_toml_config(None);
    api_impl::set_current_manifest_path(None);
    ret
}

unsafe fn read_display_name(handle: *mut std::ffi::c_void) -> Option<String> {
    let name_fn_ptr = GetProcAddress(handle, b"chumod_name\0".as_ptr())?;
    let name_fn: ChuModNameFunc = std::mem::transmute(name_fn_ptr);
    let raw = name_fn();
    if raw.is_null() {
        None
    } else {
        Some(CStr::from_ptr(raw).to_string_lossy().into_owned())
    }
}

fn mod_matches(loaded: &LoadedMod, wanted: &str) -> bool {
    normalize_name(&loaded.name) == wanted
        || normalize_name(&loaded.file_name) == wanted
        || normalize_name(&file_stem(&loaded.file_name)) == wanted
}

fn normalize_name(name: &str) -> String {
    name.trim()
        .trim_end_matches(".dll")
        .trim_end_matches(".DLL")
        .to_ascii_lowercase()
}

fn file_stem(file_name: &str) -> String {
    file_name
        .strip_suffix(".dll")
        .or_else(|| file_name.strip_suffix(".DLL"))
        .unwrap_or(file_name)
        .to_string()
}
