use super::*;

pub unsafe fn install_early(game_base: usize) {
    if HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let mut freq = 0i64;
    QueryPerformanceFrequency(&mut freq);
    QPC_FREQ.store(freq as usize, Ordering::SeqCst);

    match hook_iat(
        game_base,
        D3D9_DLL_NAME,
        DIRECT3D_CREATE9_NAME,
        hooked_direct3d_create9 as *const (),
    ) {
        Some(original) => {
            ORIGINAL_DIRECT3D_CREATE9.store(original as usize, Ordering::SeqCst);
            log_info("d3d9: Direct3DCreate9 IAT hook installed");
        }
        None => {
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            log_warn("d3d9: Direct3DCreate9 import not found (IAT hook skipped)");
        }
    }
}

unsafe extern "system" fn hooked_direct3d_create9(sdk_version: u32) -> *mut c_void {
    let addr = ORIGINAL_DIRECT3D_CREATE9.load(Ordering::SeqCst);
    if addr == 0 {
        return std::ptr::null_mut();
    }
    let original: Direct3DCreate9Fn = std::mem::transmute(addr);
    let d3d = original(sdk_version);
    if !d3d.is_null() {
        hook_create_device(d3d);
    }
    d3d
}

unsafe fn hook_create_device(d3d: *mut c_void) {
    if ORIGINAL_CREATE_DEVICE.load(Ordering::SeqCst) != 0 {
        return;
    }
    let slot = vtable_slot(d3d, D3D9_CREATE_DEVICE_INDEX);
    if slot.is_null() {
        return;
    }
    let current = *slot;
    let detour = hooked_create_device as *const () as usize;
    if current == detour {
        return;
    }
    if patch_slot(slot, detour) {
        ORIGINAL_CREATE_DEVICE.store(current, Ordering::SeqCst);
        log_info("d3d9: IDirect3D9::CreateDevice vtable hook installed");
    } else {
        log_warn("d3d9: CreateDevice vtable write failed");
    }
}

unsafe extern "system" fn hooked_create_device(
    this: *mut c_void,
    adapter: u32,
    device_type: u32,
    focus_window: usize,
    behavior_flags: u32,
    presentation_parameters: *mut c_void,
    returned_device_interface: *mut *mut c_void,
) -> i32 {
    let addr = ORIGINAL_CREATE_DEVICE.load(Ordering::SeqCst);
    if addr == 0 {
        return -1;
    }
    let original: CreateDeviceFn = std::mem::transmute(addr);
    let result = original(
        this,
        adapter,
        device_type,
        focus_window,
        behavior_flags,
        presentation_parameters,
        returned_device_interface,
    );

    if result >= 0 && !returned_device_interface.is_null() {
        let device = *returned_device_interface;
        if !device.is_null() {
            GAME_HWND.store(focus_window, Ordering::SeqCst);
            hook_device(device, presentation_parameters);
        }
    }
    result
}

unsafe fn hook_device(device: *mut c_void, presentation_parameters: *mut c_void) {
    if DEVICE_HOOKED.swap(true, Ordering::SeqCst) {
        return;
    }
    DEVICE_PTR.store(device as usize, Ordering::SeqCst);
    PRESENTATION_PARAMETERS.store(presentation_parameters as usize, Ordering::SeqCst);

    remember_slot(
        device,
        DEVICE_TEST_COOPERATIVE_LEVEL_INDEX,
        &ORIGINAL_TEST_COOPERATIVE_LEVEL,
    );

    hook_slot(
        device,
        DEVICE_RESET_INDEX,
        runtime::hooked_reset as *const () as usize,
        &ORIGINAL_RESET,
        "Reset",
    );
    hook_slot(
        device,
        DEVICE_PRESENT_INDEX,
        runtime::hooked_present as *const () as usize,
        &ORIGINAL_PRESENT,
        "Present",
    );
    hook_slot(
        device,
        DEVICE_END_SCENE_INDEX,
        runtime::hooked_end_scene as *const () as usize,
        &ORIGINAL_END_SCENE,
        "EndScene",
    );
}

unsafe fn remember_slot(device: *mut c_void, index: usize, original: &AtomicUsize) {
    if original.load(Ordering::SeqCst) != 0 {
        return;
    }
    let slot = vtable_slot(device, index);
    if !slot.is_null() {
        original.store(*slot, Ordering::SeqCst);
    }
}

unsafe fn hook_slot(
    device: *mut c_void,
    index: usize,
    detour: usize,
    original: &AtomicUsize,
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
        log_info(&format!(
            "d3d9: IDirect3DDevice9::{name} vtable hook installed"
        ));
    } else {
        log_warn(&format!("d3d9: {name} vtable write failed"));
    }
}
