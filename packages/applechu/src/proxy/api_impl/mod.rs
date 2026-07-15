mod config_ini;
mod config_toml;
mod hook;
mod ipc;
mod log_api;
mod manifest;
mod memory;
mod rtti;

use chu_abi::ChuModAPI;

pub use config_ini::set_current_config;
pub use config_toml::load_current_config as load_current_toml_config;
pub use manifest::set_current_manifest_path;
pub use rtti::set_rtti_info;

static mut G_API: ChuModAPI = ChuModAPI {
    struct_size: 0,
    log: None,
    aob_scan: None,
    mem_read: None,
    mem_write: None,
    mem_fill: None,
    hook_create: None,
    hook_enable: None,
    hook_disable: None,
    hook_remove: None,
    register_service: None,
    get_service: None,
    publish: None,
    subscribe: None,
    rtti_find_vtable: None,
    config_get_int: None,
    config_get_float: None,
    config_get_bool: None,
    config_get_string: None,
    config_set_int: None,
    config_set_float: None,
    config_set_bool: None,
    config_set_string: None,
    log_info: None,
    log_warn: None,
    log_error: None,
    log_path: std::ptr::null(),
    toml_section_exists: None,
    toml_get_bool: None,
    toml_get_int: None,
    toml_get_float: None,
    toml_get_string: None,
    get_manifest_path: None,
    reload_mod: None,
    register_present_callback: None,
    register_reset_callback: None,
    set_frame_lock: None,
    get_d3d9_device: None,
    get_game_hwnd: None,
};

fn make_api() -> ChuModAPI {
    ChuModAPI {
        struct_size: std::mem::size_of::<ChuModAPI>() as u32,
        log: None,
        aob_scan: Some(memory::api_aob_scan),
        mem_read: Some(memory::api_mem_read),
        mem_write: Some(memory::api_mem_write),
        mem_fill: Some(memory::api_mem_fill),
        hook_create: Some(hook::api_hook_create),
        hook_enable: Some(hook::api_hook_enable),
        hook_disable: Some(hook::api_hook_disable),
        hook_remove: Some(hook::api_hook_remove),
        register_service: Some(ipc::api_register_service),
        get_service: Some(ipc::api_get_service),
        publish: Some(ipc::api_publish),
        subscribe: Some(ipc::api_subscribe),
        rtti_find_vtable: Some(rtti::api_rtti_find_vtable),
        config_get_int: Some(config_ini::api_config_get_int),
        config_get_float: Some(config_ini::api_config_get_float),
        config_get_bool: Some(config_ini::api_config_get_bool),
        config_get_string: Some(config_ini::api_config_get_string),
        config_set_int: Some(config_ini::api_config_set_int),
        config_set_float: Some(config_ini::api_config_set_float),
        config_set_bool: Some(config_ini::api_config_set_bool),
        config_set_string: Some(config_ini::api_config_set_string),
        log_info: Some(log_api::api_log_info),
        log_warn: Some(log_api::api_log_warn),
        log_error: Some(log_api::api_log_error),
        log_path: std::ptr::null(),
        toml_section_exists: Some(config_toml::api_toml_section_exists),
        toml_get_bool: Some(config_toml::api_toml_get_bool),
        toml_get_int: Some(config_toml::api_toml_get_int),
        toml_get_float: Some(config_toml::api_toml_get_float),
        toml_get_string: Some(config_toml::api_toml_get_string),
        get_manifest_path: Some(manifest::api_get_manifest_path),
        reload_mod: Some(crate::proxy::loader::hot_reload::api_reload_mod),
        register_present_callback: Some(crate::proxy::d3d9::api_register_present_callback),
        register_reset_callback: Some(crate::proxy::d3d9::api_register_reset_callback),
        set_frame_lock: Some(crate::proxy::d3d9::api_set_frame_lock),
        get_d3d9_device: Some(crate::proxy::d3d9::api_get_d3d9_device),
        get_game_hwnd: Some(crate::proxy::d3d9::api_get_game_hwnd),
    }
}

pub unsafe fn set_log_path(path: *const std::ffi::c_char) {
    G_API.log_path = path;
}

pub unsafe fn init() {
    G_API = make_api();
}

pub unsafe fn shutdown() {
    hook::shutdown();
    ipc::shutdown();
}

pub unsafe fn get_api() -> *mut ChuModAPI {
    &raw mut G_API
}
