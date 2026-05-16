use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use crate::d3d9::hook::patch_slot;
use crate::util::api::API;

type PresentFn = unsafe extern "system" fn(
    *mut c_void,
    *const c_void,
    *const c_void,
    usize,
    *const c_void,
) -> i32;
type ResetFn = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type TestCooperativeLevelFn = unsafe extern "system" fn(*mut c_void) -> i32;

const D3DERR_DEVICELOST: i32 = 0x8876_0868u32 as i32;
const D3DERR_DEVICENOTRESET: i32 = 0x8876_0869u32 as i32;
const D3D_OK: i32 = 0;

const DEVICE_TEST_COOPERATIVE_LEVEL_INDEX: usize = 3;
const DEVICE_RESET_INDEX: usize = 16;
const DEVICE_PRESENT_INDEX: usize = 17;
const MAX_RECOVERY_ATTEMPTS: usize = 600;
const RECOVERY_WAIT_MS: u64 = 50;

static ORIGINAL_PRESENT: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_RESET: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_TEST_COOPERATIVE_LEVEL: AtomicUsize = AtomicUsize::new(0);
static PRESENT_SLOT: AtomicUsize = AtomicUsize::new(0);
static RESET_SLOT: AtomicUsize = AtomicUsize::new(0);
static PRESENTATION_PARAMETERS: AtomicUsize = AtomicUsize::new(0);
static RECOVERING: AtomicBool = AtomicBool::new(false);

pub fn hook_device(device: *mut c_void, presentation_parameters: *mut c_void) {
    if device.is_null() {
        return;
    }

    PRESENTATION_PARAMETERS.store(presentation_parameters as usize, Ordering::SeqCst);

    unsafe {
        remember_test_cooperative_level(device);
        hook_device_slot(
            device,
            DEVICE_RESET_INDEX,
            hooked_reset as *const () as usize,
            &ORIGINAL_RESET,
            &RESET_SLOT,
            "Reset",
        );
        hook_device_slot(
            device,
            DEVICE_PRESENT_INDEX,
            hooked_present as *const () as usize,
            &ORIGINAL_PRESENT,
            &PRESENT_SLOT,
            "Present",
        );
    }
}

unsafe extern "system" fn hooked_present(
    this: *mut c_void,
    source_rect: *const c_void,
    dest_rect: *const c_void,
    dest_window_override: usize,
    dirty_region: *const c_void,
) -> i32 {
    let Some(original) = present() else {
        return D3DERR_DEVICELOST;
    };

    let result = original(this, source_rect, dest_rect, dest_window_override, dirty_region);
    if result == D3DERR_DEVICELOST {
        recover_device(this);
    }
    result
}

unsafe extern "system" fn hooked_reset(this: *mut c_void, presentation_parameters: *mut c_void) -> i32 {
    if !presentation_parameters.is_null() {
        PRESENTATION_PARAMETERS.store(presentation_parameters as usize, Ordering::SeqCst);
    }

    let Some(original) = reset() else {
        return D3DERR_DEVICELOST;
    };

    original(this, presentation_parameters)
}

unsafe fn recover_device(device: *mut c_void) {
    if device.is_null() || RECOVERING.swap(true, Ordering::SeqCst) {
        return;
    }

    log_info("DeviceLostFix 检测到 D3DERR_DEVICELOST，开始等待 Reset");

    for _ in 0..MAX_RECOVERY_ATTEMPTS {
        let state = test_cooperative_level()
            .map(|func| func(device))
            .unwrap_or(D3DERR_DEVICELOST);

        if state == D3DERR_DEVICENOTRESET {
            let params = PRESENTATION_PARAMETERS.load(Ordering::SeqCst) as *mut c_void;
            if !params.is_null() {
                if let Some(reset) = reset() {
                    let reset_result = reset(device, params);
                    if reset_result == D3D_OK {
                        log_info("DeviceLostFix 已通过 Reset 恢复 D3D9 device");
                    } else {
                        log_warn("DeviceLostFix 调用 Reset 后仍未恢复");
                    }
                }
            }
            break;
        }

        if state == D3D_OK {
            break;
        }

        thread::sleep(Duration::from_millis(RECOVERY_WAIT_MS));
    }

    RECOVERING.store(false, Ordering::SeqCst);
}

unsafe fn remember_test_cooperative_level(device: *mut c_void) {
    if ORIGINAL_TEST_COOPERATIVE_LEVEL.load(Ordering::SeqCst) != 0 {
        return;
    }

    let slot = vtable_slot(device, DEVICE_TEST_COOPERATIVE_LEVEL_INDEX);
    if !slot.is_null() {
        ORIGINAL_TEST_COOPERATIVE_LEVEL.store(*slot, Ordering::SeqCst);
    }
}

unsafe fn hook_device_slot(
    device: *mut c_void,
    index: usize,
    detour: usize,
    original: &AtomicUsize,
    stored_slot: &AtomicUsize,
    name: &str,
) {
    let slot = vtable_slot(device, index);
    if slot.is_null() {
        return;
    }

    let current = *slot;
    if current == detour || original.load(Ordering::SeqCst) != 0 {
        return;
    }

    if patch_slot(slot, detour) {
        original.store(current, Ordering::SeqCst);
        stored_slot.store(slot as usize, Ordering::SeqCst);
        log_info(&format!("DeviceLostFix 已安装: IDirect3DDevice9::{name} vtable hook"));
    } else {
        log_warn(&format!("DeviceLostFix 初始化失败: {name} vtable 写入失败"));
    }
}

unsafe fn vtable_slot(instance: *mut c_void, index: usize) -> *mut usize {
    if instance.is_null() {
        return std::ptr::null_mut();
    }

    let vtable = *(instance as *const *mut usize);
    if vtable.is_null() {
        return std::ptr::null_mut();
    }

    vtable.add(index)
}

unsafe fn present() -> Option<PresentFn> {
    let addr = ORIGINAL_PRESENT.load(Ordering::SeqCst);
    (addr != 0).then(|| std::mem::transmute(addr))
}

unsafe fn reset() -> Option<ResetFn> {
    let addr = ORIGINAL_RESET.load(Ordering::SeqCst);
    (addr != 0).then(|| std::mem::transmute(addr))
}

unsafe fn test_cooperative_level() -> Option<TestCooperativeLevelFn> {
    let addr = ORIGINAL_TEST_COOPERATIVE_LEVEL.load(Ordering::SeqCst);
    (addr != 0).then(|| std::mem::transmute(addr))
}

fn log_info(message: &str) {
    if let Some(api) = API.get() {
        api.log_info(message);
    }
}

fn log_warn(message: &str) {
    if let Some(api) = API.get() {
        api.log_warn(message);
    }
}
