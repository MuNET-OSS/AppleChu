mod hooks;
mod iat_hook;
mod runtime;

pub use hooks::install_early;
pub use runtime::{
    api_get_d3d9_device, api_get_game_hwnd, api_register_present_callback,
    api_register_reset_callback, api_set_frame_lock,
};

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use chu_abi::{ChuModPresentCallback, ChuModResetCallback};

use crate::proxy::loader::log::{log_info, log_warn};

use self::iat_hook::{hook_iat, patch_slot, vtable_slot};

const D3D9_DLL_NAME: &str = "d3d9.dll";
const DIRECT3D_CREATE9_NAME: &str = "Direct3DCreate9";

const D3D9_CREATE_DEVICE_INDEX: usize = 16;
const DEVICE_TEST_COOPERATIVE_LEVEL_INDEX: usize = 3;
const DEVICE_RESET_INDEX: usize = 16;
const DEVICE_PRESENT_INDEX: usize = 17;
const DEVICE_END_SCENE_INDEX: usize = 42;

const D3DERR_DEVICELOST: i32 = 0x8876_0868u32 as i32;
const D3DERR_DEVICENOTRESET: i32 = 0x8876_0869u32 as i32;
const D3D_OK: i32 = 0;
const MAX_RECOVERY_ATTEMPTS: usize = 600;
const RECOVERY_WAIT_MS: u64 = 50;

const MAX_PRESENT_CALLBACKS: usize = 8;
const MAX_RESET_CALLBACKS: usize = 8;

type Direct3DCreate9Fn = unsafe extern "system" fn(u32) -> *mut c_void;
type CreateDeviceFn = unsafe extern "system" fn(
    *mut c_void,
    u32,
    u32,
    usize,
    u32,
    *mut c_void,
    *mut *mut c_void,
) -> i32;
type PresentFn = unsafe extern "system" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    usize,
    *const c_void,
) -> i32;
type ResetFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type EndSceneFn = unsafe extern "system" fn(*mut c_void) -> i32;
type TestCooperativeLevelFn = unsafe extern "system" fn(*mut c_void) -> i32;

static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
static ORIGINAL_DIRECT3D_CREATE9: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CREATE_DEVICE: AtomicUsize = AtomicUsize::new(0);
static DEVICE_HOOKED: AtomicBool = AtomicBool::new(false);

static ORIGINAL_PRESENT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESET: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_END_SCENE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TEST_COOPERATIVE_LEVEL: AtomicUsize = AtomicUsize::new(0);

static DEVICE_PTR: AtomicUsize = AtomicUsize::new(0);
static GAME_HWND: AtomicUsize = AtomicUsize::new(0);
static PRESENTATION_PARAMETERS: AtomicUsize = AtomicUsize::new(0);
static RECOVERING: AtomicBool = AtomicBool::new(false);

static FRAME_LOCK_FPS: AtomicU32 = AtomicU32::new(0);
static QPC_FREQ: AtomicUsize = AtomicUsize::new(0);
static LAST_FRAME_QPC: AtomicUsize = AtomicUsize::new(0);

static PRESENT_CALLBACKS: [AtomicUsize; MAX_PRESENT_CALLBACKS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];
static RESET_CALLBACKS: [AtomicUsize; MAX_RESET_CALLBACKS] = [
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
    AtomicUsize::new(0),
];

#[link(name = "kernel32")]
extern "system" {
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(freq: *mut i64) -> i32;
}
