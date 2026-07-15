//! ChuMod ABI
//!
//! AppleChu 与外部 Mod 共同依赖此 crate
//!
//! ABI 安全约束 ：
//! - 跨边界结构 `#[repr(C)]`，仅用定宽整型 / 裸指针；
//! - 禁止跨边界使用 Rust `bool`/`Vec`/`String`/slice/trait object/无显式 repr 枚举；
//! - chumod 回调表统一 `extern "C"`（i686 cdecl），不可改 stdcall，否则爆栈；
//! - 仅 append-only 增长：永不重排字段、不缩类型、不改回调签名；
//! - 新字段靠 `ChuModAPI::struct_size` 向后兼容

#![no_std]
#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_void};

/// 当前 ABI 版本 v4：d3d9 服务
pub const CHUMOD_API_VERSION: u32 = 4;

/// Loader 版本
pub const LOADER_VERSION: &str = "1.0.0";

/// 传给 `chumod_init` 的运行时信息（append-only）
#[repr(C)]
pub struct ChuModInfo {
    pub api_version: u32,
    pub loader_version: *const c_char,
    pub game_module: *const c_char,
    pub game_base: usize,
    pub game_size: u32,
    pub text_base: usize,
    pub text_size: u32,
    pub rdata_base: usize,
    pub rdata_size: u32,
}

pub type ChuModInitFunc = unsafe extern "C" fn(*const ChuModInfo, *const ChuModAPI) -> i32;
pub type ChuModReadyFunc = unsafe extern "C" fn();
pub type ChuModFrameFunc = unsafe extern "C" fn();
pub type ChuModShutdownFunc = unsafe extern "C" fn();
pub type ChuModNameFunc = unsafe extern "C" fn() -> *const c_char;
pub type ChuModDependsFunc = unsafe extern "C" fn() -> *const c_char;
pub type ChuModStringFunc = unsafe extern "C" fn() -> *const c_char;

/// 每次 Present 调用（FPS OSD 等） 参数为原生 `IDirect3DDevice9*`
pub type ChuModPresentCallback = unsafe extern "C" fn(device: *mut c_void);
/// 设备 Reset 事件 `phase`: 0 = Reset 前（释放 D3DPOOL_DEFAULT），1 = Reset 成功后（重建）
pub type ChuModResetCallback = unsafe extern "C" fn(device: *mut c_void, phase: u32);

/// Loader 提供给 mod 的 API 函数表（append-only；用 `struct_size` 判断可用字段）
#[repr(C)]
pub struct ChuModAPI {
    pub struct_size: u32,

    pub log: Option<unsafe extern "C" fn(*const c_char, ...)>,

    pub aob_scan: Option<unsafe extern "C" fn(usize, u32, *const u8, *const c_char) -> usize>,
    pub mem_read: Option<unsafe extern "C" fn(usize, *mut c_void, u32) -> i32>,
    pub mem_write: Option<unsafe extern "C" fn(usize, *const c_void, u32) -> i32>,
    pub mem_fill: Option<unsafe extern "C" fn(usize, u8, u32) -> i32>,

    pub hook_create:
        Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> i32>,
    pub hook_enable: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub hook_disable: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    pub hook_remove: Option<unsafe extern "C" fn(*mut c_void) -> i32>,

    pub register_service: Option<unsafe extern "C" fn(*const c_char, *mut c_void) -> i32>,
    pub get_service: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
    pub publish: Option<unsafe extern "C" fn(*const c_char, *mut c_void, u32) -> i32>,
    pub subscribe: Option<
        unsafe extern "C" fn(
            *const c_char,
            Option<unsafe extern "C" fn(*const c_char, *mut c_void, u32)>,
        ) -> i32,
    >,

    pub rtti_find_vtable: Option<unsafe extern "C" fn(*const c_char) -> usize>,

    pub config_get_int: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_get_float: Option<unsafe extern "C" fn(*const c_char, f32) -> f32>,
    pub config_get_bool: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_get_string:
        Option<unsafe extern "C" fn(*const c_char, *mut c_char, u32, *const c_char) -> i32>,
    pub config_set_int: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_set_float: Option<unsafe extern "C" fn(*const c_char, f32) -> i32>,
    pub config_set_bool: Option<unsafe extern "C" fn(*const c_char, i32) -> i32>,
    pub config_set_string: Option<unsafe extern "C" fn(*const c_char, *const c_char) -> i32>,

    pub log_info: Option<unsafe extern "C" fn(*const c_char)>,
    pub log_warn: Option<unsafe extern "C" fn(*const c_char)>,
    pub log_error: Option<unsafe extern "C" fn(*const c_char)>,

    pub log_path: *const c_char,

    pub toml_section_exists: Option<unsafe extern "C" fn(*const c_char) -> i32>,
    pub toml_get_bool: Option<unsafe extern "C" fn(*const c_char, *const c_char, i32) -> i32>,
    pub toml_get_int: Option<unsafe extern "C" fn(*const c_char, *const c_char, i32) -> i32>,
    pub toml_get_float: Option<unsafe extern "C" fn(*const c_char, *const c_char, f32) -> f32>,
    pub toml_get_string: Option<
        unsafe extern "C" fn(*const c_char, *const c_char, *mut c_char, u32, *const c_char) -> i32,
    >,

    pub get_manifest_path: Option<unsafe extern "C" fn() -> *const c_char>,

    pub reload_mod: Option<unsafe extern "C" fn(*const c_char) -> i32>,

    pub register_present_callback:
        Option<unsafe extern "C" fn(Option<ChuModPresentCallback>) -> i32>,
    pub register_reset_callback: Option<unsafe extern "C" fn(Option<ChuModResetCallback>) -> i32>,
    pub set_frame_lock: Option<unsafe extern "C" fn(u32) -> i32>,
    pub get_d3d9_device: Option<unsafe extern "C" fn() -> *mut c_void>,
    pub get_game_hwnd: Option<unsafe extern "C" fn() -> usize>,
}
