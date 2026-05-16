use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::d3d9::recovery;
use crate::util::api::{Api, API};
use crate::util::iat_hook::hook_iat;

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

const DIRECT3D_CREATE9_NAME: &str = "Direct3DCreate9";
const D3D9_DLL_NAME: &str = "d3d9.dll";
const DIRECT3D9_CREATE_DEVICE_INDEX: usize = 16;

static ORIGINAL_DIRECT3D_CREATE9: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CREATE_DEVICE: AtomicUsize = AtomicUsize::new(0);
static CREATE_DEVICE_SLOT: AtomicUsize = AtomicUsize::new(0);
static HOOKED: AtomicBool = AtomicBool::new(false);

pub fn init(api: &Api) {
    if HOOKED.swap(true, Ordering::SeqCst) {
        return;
    }

    let original = unsafe {
        hook_iat(
            api.game_base(),
            D3D9_DLL_NAME,
            DIRECT3D_CREATE9_NAME,
            hooked_direct3d_create9 as *const (),
        )
    };

    let Some(original) = original else {
        HOOKED.store(false, Ordering::SeqCst);
        api.log_warn("DeviceLostFix 初始化跳过: 未找到 Direct3DCreate9 导入");
        return;
    };

    ORIGINAL_DIRECT3D_CREATE9.store(original as usize, Ordering::SeqCst);
    api.log_info("DeviceLostFix 已安装: Direct3DCreate9 IAT hook");
}

unsafe extern "system" fn hooked_direct3d_create9(sdk_version: u32) -> *mut c_void {
    let Some(original) = direct3d_create9() else {
        return std::ptr::null_mut();
    };

    let d3d = original(sdk_version);
    if !d3d.is_null() {
        hook_create_device(d3d);
    }
    d3d
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
    let Some(original) = create_device() else {
        return -1;
    };

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
            recovery::hook_device(device, presentation_parameters);
        }
    }

    result
}

fn hook_create_device(d3d: *mut c_void) {
    let slot = unsafe { vtable_slot(d3d, DIRECT3D9_CREATE_DEVICE_INDEX) };
    if slot.is_null() {
        return;
    }

    let current = unsafe { *slot };
    let detour = hooked_create_device as *const () as usize;
    if current == detour || ORIGINAL_CREATE_DEVICE.load(Ordering::SeqCst) != 0 {
        return;
    }

    if unsafe { patch_slot(slot, detour) } {
        ORIGINAL_CREATE_DEVICE.store(current, Ordering::SeqCst);
        CREATE_DEVICE_SLOT.store(slot as usize, Ordering::SeqCst);
        log_info("DeviceLostFix 已安装: IDirect3D9::CreateDevice vtable hook");
    } else {
        log_warn("DeviceLostFix 初始化失败: CreateDevice vtable 写入失败");
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

pub(crate) unsafe fn patch_slot(slot: *mut usize, value: usize) -> bool {
    const PAGE_EXECUTE_READWRITE: u32 = 0x40;

    #[link(name = "kernel32")]
    extern "system" {
        fn VirtualProtect(address: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    }

    if slot.is_null() || value == 0 {
        return false;
    }

    let mut old_protect = 0;
    if VirtualProtect(
        slot.cast(),
        std::mem::size_of::<usize>(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return false;
    }

    *slot = value;

    let mut ignored = 0;
    let _ = VirtualProtect(slot.cast(), std::mem::size_of::<usize>(), old_protect, &mut ignored);
    true
}

unsafe fn direct3d_create9() -> Option<Direct3DCreate9Fn> {
    let addr = ORIGINAL_DIRECT3D_CREATE9.load(Ordering::SeqCst);
    (addr != 0).then(|| std::mem::transmute(addr))
}

unsafe fn create_device() -> Option<CreateDeviceFn> {
    let addr = ORIGINAL_CREATE_DEVICE.load(Ordering::SeqCst);
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
