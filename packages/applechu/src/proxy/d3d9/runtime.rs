use super::*;

pub(super) unsafe extern "system" fn hooked_present(
    this: *mut c_void,
    source_rect: *const c_void,
    dest_rect: *const c_void,
    dest_window_override: usize,
    dirty_region: *const c_void,
) -> i32 {
    frame_lock_wait();
    run_present_callbacks(this);

    let addr = ORIGINAL_PRESENT.load(Ordering::SeqCst);
    if addr == 0 {
        return D3DERR_DEVICELOST;
    }
    let original: PresentFn = std::mem::transmute(addr);
    let result = original(
        this,
        source_rect,
        dest_rect,
        dest_window_override,
        dirty_region,
    );
    if result == D3DERR_DEVICELOST {
        recover_device(this);
    }
    result
}

pub(super) unsafe extern "system" fn hooked_end_scene(this: *mut c_void) -> i32 {
    let addr = ORIGINAL_END_SCENE.load(Ordering::SeqCst);
    if addr == 0 {
        return D3D_OK;
    }
    let original: EndSceneFn = std::mem::transmute(addr);
    original(this)
}

pub(super) unsafe extern "system" fn hooked_reset(
    this: *mut c_void,
    presentation_parameters: *mut c_void,
) -> i32 {
    if !presentation_parameters.is_null() {
        PRESENTATION_PARAMETERS.store(presentation_parameters as usize, Ordering::SeqCst);
    }
    run_reset_callbacks(this, 0);

    let addr = ORIGINAL_RESET.load(Ordering::SeqCst);
    if addr == 0 {
        return D3DERR_DEVICELOST;
    }
    let original: ResetFn = std::mem::transmute(addr);
    let result = original(this, presentation_parameters);
    if result == D3D_OK {
        run_reset_callbacks(this, 1);
    }
    result
}

unsafe fn recover_device(device: *mut c_void) {
    if device.is_null() || RECOVERING.swap(true, Ordering::SeqCst) {
        return;
    }
    log_info("D3D9 device lost; waiting for recovery");

    let test_addr = ORIGINAL_TEST_COOPERATIVE_LEVEL.load(Ordering::SeqCst);
    let reset_addr = ORIGINAL_RESET.load(Ordering::SeqCst);

    for _ in 0..MAX_RECOVERY_ATTEMPTS {
        let state = if test_addr != 0 {
            let test: TestCooperativeLevelFn = std::mem::transmute(test_addr);
            test(device)
        } else {
            D3DERR_DEVICELOST
        };

        if state == D3DERR_DEVICENOTRESET {
            let params = PRESENTATION_PARAMETERS.load(Ordering::SeqCst) as *mut c_void;
            if !params.is_null() && reset_addr != 0 {
                run_reset_callbacks(device, 0);
                let reset: ResetFn = std::mem::transmute(reset_addr);
                if reset(device, params) == D3D_OK {
                    run_reset_callbacks(device, 1);
                    log_info("D3D9 device recovered");
                } else {
                    log_warn("D3D9 device recovery failed");
                }
            }
            break;
        }

        if state == D3D_OK {
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(RECOVERY_WAIT_MS));
    }

    RECOVERING.store(false, Ordering::SeqCst);
}

unsafe fn run_present_callbacks(device: *mut c_void) {
    for slot in &PRESENT_CALLBACKS {
        let addr = slot.load(Ordering::SeqCst);
        if addr != 0 {
            let cb: ChuModPresentCallback = std::mem::transmute(addr);
            cb(device);
        }
    }
}

unsafe fn run_reset_callbacks(device: *mut c_void, phase: u32) {
    for slot in &RESET_CALLBACKS {
        let addr = slot.load(Ordering::SeqCst);
        if addr != 0 {
            let cb: ChuModResetCallback = std::mem::transmute(addr);
            cb(device, phase);
        }
    }
}

unsafe fn frame_lock_wait() {
    let fps = FRAME_LOCK_FPS.load(Ordering::SeqCst);
    let freq = QPC_FREQ.load(Ordering::SeqCst) as u64;
    if fps == 0 || freq == 0 {
        return;
    }
    let target_interval = freq / fps as u64;
    let last = LAST_FRAME_QPC.load(Ordering::SeqCst) as u64;
    loop {
        let mut now = 0i64;
        QueryPerformanceCounter(&mut now);
        let now = now as u64;
        if now.wrapping_sub(last) >= target_interval {
            LAST_FRAME_QPC.store(now as usize, Ordering::SeqCst);
            return;
        }
        std::hint::spin_loop();
    }
}

pub unsafe extern "C" fn api_register_present_callback(
    callback: Option<ChuModPresentCallback>,
) -> i32 {
    let Some(cb) = callback else { return -1 };
    let value = cb as usize;
    for slot in &PRESENT_CALLBACKS {
        if slot
            .compare_exchange(0, value, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return 0;
        }
    }
    -1
}

pub unsafe extern "C" fn api_register_reset_callback(callback: Option<ChuModResetCallback>) -> i32 {
    let Some(cb) = callback else { return -1 };
    let value = cb as usize;
    for slot in &RESET_CALLBACKS {
        if slot
            .compare_exchange(0, value, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return 0;
        }
    }
    -1
}

pub unsafe extern "C" fn api_set_frame_lock(fps: u32) -> i32 {
    FRAME_LOCK_FPS.store(fps, Ordering::SeqCst);
    0
}

pub unsafe extern "C" fn api_get_d3d9_device() -> *mut c_void {
    DEVICE_PTR.load(Ordering::SeqCst) as *mut c_void
}

pub unsafe extern "C" fn api_get_game_hwnd() -> usize {
    GAME_HWND.load(Ordering::SeqCst)
}
