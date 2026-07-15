use std::ffi::CStr;
use std::fs::File;

use chu_abi::{
    ChuModAPI, ChuModFrameFunc, ChuModInfo, ChuModInitFunc, ChuModNameFunc, ChuModReadyFunc,
    ChuModShutdownFunc,
};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

use crate::proxy::api_impl;

use super::dependency::{read_dependencies, sort_mods, PendingMod};
use super::log::{log_info, write_log_inner, write_log_variadic};
use super::metadata::{read_metadata, should_load_metadata};
use super::scanner::{ensure_mods_dir, scan_manifest_files, scan_mod_files};
use super::seh::call_mod_init;
use super::state::{LoadedMod, STATE};
use super::FreeLibrary;

pub(super) unsafe fn load(base_dir: &str, info: &ChuModInfo, api: *mut ChuModAPI) {
    let mods_dir = ensure_mods_dir(base_dir);
    log_info(&format!("scan mods dir: {mods_dir}"));
    let manifests = scan_manifest_files(base_dir);
    {
        let mut state = STATE.lock().unwrap();
        state.manifest_paths = manifests.clone();
    }
    for manifest in &manifests {
        log_info(&format!("manifest found: {manifest}"));
    }

    let mut pending_mods = Vec::new();
    for (mod_name, full_path) in scan_mod_files(&mods_dir) {
        if !should_load_external_mod(&mod_name) {
            log_info("skip bundled mod: AppleChu");
            continue;
        }
        let full_path_c = format!("{full_path}\0");
        let mod_handle = LoadLibraryA(full_path_c.as_ptr());
        if mod_handle.is_null() {
            log_info(&format!(
                "failed to load mod: {full_path} (err={})",
                GetLastError()
            ));
            continue;
        }

        let mut display_name = mod_name.clone();
        let name_fn_ptr = GetProcAddress(mod_handle, b"chumod_name\0".as_ptr());
        if let Some(name_fn) = name_fn_ptr {
            let name_fn: ChuModNameFunc = std::mem::transmute(name_fn);
            let name = name_fn();
            if !name.is_null() {
                display_name = CStr::from_ptr(name).to_string_lossy().into_owned();
            }
        }

        let metadata = read_metadata(mod_handle);
        if !should_load_metadata(&display_name, &metadata) {
            FreeLibrary(mod_handle);
            continue;
        }
        if let Some(version) = &metadata.version {
            log_info(&format!("mod: {display_name} v{version}"));
        }

        let dependencies = read_dependencies(mod_handle);
        pending_mods.push(PendingMod {
            file_name: mod_name,
            full_path,
            handle: mod_handle,
            display_name,
            dependencies,
        });
    }

    for pending in sort_mods(pending_mods) {
        let mod_name = pending.file_name;
        let full_path = pending.full_path;
        let mod_handle = pending.handle;
        let display_name = pending.display_name;

        let init_fn_ptr = GetProcAddress(mod_handle, b"chumod_init\0".as_ptr());
        if let Some(init_fn) = init_fn_ptr {
            let init_fn: ChuModInitFunc = std::mem::transmute(init_fn);
            let config_dir = format!("{base_dir}\\mods\\config");
            let log_dir = format!("{base_dir}\\mods\\log");
            let mod_stem = mod_name
                .strip_suffix(".dll")
                .or_else(|| mod_name.strip_suffix(".DLL"))
                .unwrap_or(&mod_name);
            let mod_log_path = format!("{log_dir}\\{mod_stem}.log");
            let mod_log_path_c = std::ffi::CString::new(mod_log_path.clone()).unwrap_or_default();
            let toml_config_path = format!("{config_dir}\\{mod_stem}.toml");
            let ini_config_path = format!("{config_dir}\\{mod_stem}.ini");
            let toml_config_exists = std::path::Path::new(&toml_config_path).exists();
            let manifest_path = format!("{base_dir}\\mods\\manifest\\{mod_stem}.toml");
            let manifest_exists = std::path::Path::new(&manifest_path).exists();

            (*api).log = Some(write_log_variadic);
            api_impl::set_log_path(mod_log_path_c.as_ptr());
            api_impl::set_current_config(if toml_config_exists {
                &toml_config_path
            } else {
                &ini_config_path
            });
            api_impl::load_current_toml_config(
                toml_config_exists.then_some(toml_config_path.as_str()),
            );
            api_impl::set_current_manifest_path(manifest_exists.then_some(manifest_path.as_str()));

            if let Ok(mut state) = STATE.lock() {
                state.current_mod_log_file = File::create(&mod_log_path).ok();
            }

            let ret = call_mod_init(&display_name, init_fn, info, api);

            if let Ok(mut state) = STATE.lock() {
                state.current_mod_log_file = None;
            }
            api_impl::set_log_path(std::ptr::null());
            api_impl::load_current_toml_config(None);
            api_impl::set_current_manifest_path(None);

            if ret != Some(0) {
                if let Some(ret) = ret {
                    log_info(&format!("mod init failed (ret={ret}): {display_name}"));
                }
                FreeLibrary(mod_handle);
                continue;
            }
        }

        let shutdown_ptr = GetProcAddress(mod_handle, b"chumod_shutdown\0".as_ptr());
        let shutdown: Option<ChuModShutdownFunc> = shutdown_ptr.map(|f| std::mem::transmute(f));
        let on_ready_ptr = GetProcAddress(mod_handle, b"chumod_on_ready\0".as_ptr());
        let on_ready: Option<ChuModReadyFunc> = on_ready_ptr.map(|f| std::mem::transmute(f));
        let on_frame_ptr = GetProcAddress(mod_handle, b"chumod_on_frame\0".as_ptr());
        let on_frame: Option<ChuModFrameFunc> = on_frame_ptr.map(|f| std::mem::transmute(f));

        let mut state = STATE.lock().unwrap();
        state.mods.push(LoadedMod {
            handle: mod_handle,
            on_ready,
            on_frame,
            shutdown,
            file_name: mod_name.clone(),
            full_path,
            name: display_name.clone(),
        });
        write_log_inner(&mut state, &format!("loaded mod: {display_name}"));
    }
}

fn should_load_external_mod(name: &str) -> bool {
    !name.eq_ignore_ascii_case("AppleChu.dll")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_applechu_is_not_loaded_as_external_mod() {
        // Given: mods 目录同时包含旧版 AppleChu 和其他第三方 mod。
        let names = ["AppleChu.dll", "applechu.DLL", "CustomIo.dll"];

        // When: loader 判断哪些 DLL 仍应作为外部 mod 加载。
        let selected = names
            .into_iter()
            .filter(|name| should_load_external_mod(name))
            .collect::<Vec<_>>();

        // Then: 只过滤已经内建的 AppleChu，第三方 mod 能力保持不变。
        assert_eq!(selected, ["CustomIo.dll"]);
    }
}
