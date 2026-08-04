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
            log_info("D3D9 entry hook installed");
        }
        None => {
            HOOK_INSTALLED.store(false, Ordering::SeqCst);
            log_warn("Direct3DCreate9 was not found; D3D9 compatibility was skipped");
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
        log_info("D3D9 device creation hook installed");
    } else {
        log_warn("Failed to install the D3D9 device creation hook");
    }
}

unsafe extern "system" fn hooked_create_device(
    this: *mut c_void,
    _adapter: u32,
    device_type: u32,
    focus_window: usize,
    behavior_flags: u32,
    presentation_parameters: *mut D3dPresentParameters,
    returned_device_interface: *mut *mut c_void,
) -> i32 {
    let addr = ORIGINAL_CREATE_DEVICE.load(Ordering::SeqCst);
    if addr == 0 {
        return -1;
    }
    if WINDOWED_MODE.load(Ordering::SeqCst) {
        // SAFETY: [类别 8：FFI 边界] Direct3D 保证非空参数在调用期间指向可写且正确对齐的展示参数。
        if let Some(parameters) = unsafe { presentation_parameters.as_mut() } {
            parameters.force_windowed();
        }
    }
    // 显示适配器始终使用配置值，不沿用游戏传入值
    let get_adapter_count: GetAdapterCountFn =
        std::mem::transmute(*(*(this as *const *const usize)).add(D3D9_GET_ADAPTER_COUNT_INDEX));
    let adapter_count = get_adapter_count(this);
    let configured_adapter = crate::gfx::monitor::preferred_adapter().max(0) as u32;
    let adapter = crate::gfx::monitor::select_adapter(adapter_count);
    if configured_adapter == adapter {
        log_info(&format!("D3D9 is using display adapter {adapter}"));
    } else {
        log_warn(&format!(
            "gfx: requested adapter {configured_adapter}, but adapter count is {adapter_count}; using adapter 0"
        ));
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

unsafe fn hook_device(device: *mut c_void, presentation_parameters: *mut D3dPresentParameters) {
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
        log_info(&format!("D3D9 device method hook installed: {name}"));
    } else {
        log_warn(&format!(
            "Failed to install D3D9 device method hook: {name}"
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "system" fn fake_create_device(
        _this: *mut c_void,
        _adapter: u32,
        _device_type: u32,
        _focus_window: usize,
        _behavior_flags: u32,
        _presentation_parameters: *mut D3dPresentParameters,
        _returned_device_interface: *mut *mut c_void,
    ) -> i32 {
        0
    }

    #[test]
    #[cfg_attr(miri, ignore = "Win32 vtable 函数地址通过 usize 保存")]
    fn create_device_forces_windowed_without_d3d9ex() {
        // Given: loader 直接代理普通 D3D9，游戏仍传入全屏参数。
        let previous_create_device =
            ORIGINAL_CREATE_DEVICE.swap(fake_create_device as *const () as usize, Ordering::SeqCst);
        let previous_windowed = WINDOWED_MODE.swap(true, Ordering::SeqCst);
        let mut parameters = D3dPresentParameters::default();
        parameters.full_screen_refresh_rate_in_hz = 60;

        // When: 参数经过 loader 的通用 CreateDevice hook。
        // SAFETY: [类别 8：FFI 边界] 假函数签名与 CreateDeviceFn 完全一致，参数在本次调用期间有效。
        let result = unsafe {
            hooked_create_device(
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                &mut parameters,
                std::ptr::null_mut(),
            )
        };
        ORIGINAL_CREATE_DEVICE.store(previous_create_device, Ordering::SeqCst);
        WINDOWED_MODE.store(previous_windowed, Ordering::SeqCst);

        // Then: 普通 D3D9 后端也收到窗口展示参数。
        assert_eq!(result, 0);
        assert_eq!(parameters.windowed, 1);
        assert_eq!(parameters.full_screen_refresh_rate_in_hz, 0);
    }
}
