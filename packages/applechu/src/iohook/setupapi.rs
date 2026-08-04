use std::collections::HashSet;
use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use windows_sys::Win32::Foundation::HMODULE;

use crate::iohook::{hook_table::HookSymbol, proc_addr};
use crate::util::api::Api;

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const FAKE_DEVICE_INFO_SET: usize = 0x4348_4944;
const INVALID_HANDLE_VALUE: usize = usize::MAX;
const SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE: u32 = 4;
const SP_DEVICE_INTERFACE_DETAIL_DATA_W_CB_SIZE: u32 = if cfg!(target_pointer_width = "64") {
    8
} else {
    6
};

type SetupDiGetClassDevsWFn =
    unsafe extern "system" fn(*const Guid, *const u16, usize, u32) -> usize;
type SetupDiGetClassDevsAFn =
    unsafe extern "system" fn(*const Guid, *const c_char, usize, u32) -> usize;
type SetupDiEnumDeviceInterfacesFn = unsafe extern "system" fn(
    usize,
    *mut SpDevinfoData,
    *const Guid,
    u32,
    *mut SpDeviceInterfaceData,
) -> i32;
type SetupDiGetDeviceInterfaceDetailWFn = unsafe extern "system" fn(
    usize,
    *mut SpDeviceInterfaceData,
    *mut c_void,
    u32,
    *mut u32,
    *mut SpDevinfoData,
) -> i32;
type SetupDiGetDeviceInterfaceDetailAFn = unsafe extern "system" fn(
    usize,
    *mut SpDeviceInterfaceData,
    *mut c_void,
    u32,
    *mut u32,
    *mut SpDevinfoData,
) -> i32;
type SetupDiDestroyDeviceInfoListFn = unsafe extern "system" fn(usize) -> i32;

static ORIG_GET_CLASS_DEVS: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_CLASS_DEVS_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_ENUM_INTERFACES: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_DETAIL: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_DETAIL_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_DESTROY_LIST: AtomicUsize = AtomicUsize::new(0);
static PHANTOM_SETS: Lazy<Mutex<HashSet<usize>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static PHANTOM_HID_PATH: Lazy<Mutex<Option<Vec<u16>>>> = Lazy::new(|| Mutex::new(None));

static mut ORIG_GET_CLASS_DEVS_PTR: *const () = std::ptr::null();
static mut ORIG_GET_CLASS_DEVS_A_PTR: *const () = std::ptr::null();
static mut ORIG_ENUM_INTERFACES_PTR: *const () = std::ptr::null();
static mut ORIG_GET_DETAIL_PTR: *const () = std::ptr::null();
static mut ORIG_GET_DETAIL_A_PTR: *const () = std::ptr::null();
static mut ORIG_DESTROY_LIST_PTR: *const () = std::ptr::null();

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

#[repr(C)]
struct SpDevinfoData {
    cb_size: u32,
    class_guid: Guid,
    dev_inst: u32,
    reserved: usize,
}

#[repr(C)]
struct SpDeviceInterfaceData {
    cb_size: u32,
    interface_class_guid: Guid,
    flags: u32,
    reserved: usize,
}

const GUID_DEVINTERFACE_HID: Guid = Guid {
    data1: 0x4D1E_55B2,
    data2: 0xF16F,
    data3: 0x11CF,
    data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
};

#[link(name = "kernel32")]
extern "system" {
    fn SetLastError(error: u32);
    fn GetModuleHandleA(name: *const u8) -> HMODULE;
}

pub fn init(api: &Api) {
    unsafe {
        let symbols = [
            HookSymbol {
                name: "SetupDiGetClassDevsA",
                patch: hooked_get_class_devs_a as *const (),
                original: std::ptr::addr_of_mut!(ORIG_GET_CLASS_DEVS_A_PTR),
            },
            HookSymbol {
                name: "SetupDiGetClassDevsW",
                patch: hooked_get_class_devs as *const (),
                original: std::ptr::addr_of_mut!(ORIG_GET_CLASS_DEVS_PTR),
            },
            HookSymbol {
                name: "SetupDiEnumDeviceInterfaces",
                patch: hooked_enum_device_interfaces as *const (),
                original: std::ptr::addr_of_mut!(ORIG_ENUM_INTERFACES_PTR),
            },
            HookSymbol {
                name: "SetupDiGetDeviceInterfaceDetailA",
                patch: hooked_get_device_interface_detail_a as *const (),
                original: std::ptr::addr_of_mut!(ORIG_GET_DETAIL_A_PTR),
            },
            HookSymbol {
                name: "SetupDiGetDeviceInterfaceDetailW",
                patch: hooked_get_device_interface_detail as *const (),
                original: std::ptr::addr_of_mut!(ORIG_GET_DETAIL_PTR),
            },
            HookSymbol {
                name: "SetupDiDestroyDeviceInfoList",
                patch: hooked_destroy_device_info_list as *const (),
                original: std::ptr::addr_of_mut!(ORIG_DESTROY_LIST_PTR),
            },
        ];
        proc_addr::push("setupapi.dll", &symbols, sync_originals);
        let patched = crate::iohook::hook_table::hook_table_apply(
            crate::iohook::hook_table::null_module(),
            "setupapi.dll",
            &symbols,
        );
        sync_originals();
        api.log_info(&format!(
            "Device discovery compatibility ready with {patched} patched entries"
        ));
    }
}

/// 注册由 IO4 模拟器公开的 HID 设备接口
pub fn add_phantom_hid(path: &str) -> bool {
    let mut wide = path.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    let Ok(mut registered) = PHANTOM_HID_PATH.lock() else {
        return false;
    };
    if registered.as_ref().is_some_and(|current| current == &wide) {
        return true;
    }
    if registered.is_some() {
        return false;
    }
    *registered = Some(wide);
    true
}

fn sync_originals() {
    // SAFETY: 原函数槽由 hook 安装过程写入，随后通过原子变量发布给并发调用者。
    unsafe {
        ORIG_GET_CLASS_DEVS.store(ORIG_GET_CLASS_DEVS_PTR as usize, Ordering::SeqCst);
        ORIG_GET_CLASS_DEVS_A.store(ORIG_GET_CLASS_DEVS_A_PTR as usize, Ordering::SeqCst);
        ORIG_ENUM_INTERFACES.store(ORIG_ENUM_INTERFACES_PTR as usize, Ordering::SeqCst);
        ORIG_GET_DETAIL.store(ORIG_GET_DETAIL_PTR as usize, Ordering::SeqCst);
        ORIG_GET_DETAIL_A.store(ORIG_GET_DETAIL_A_PTR as usize, Ordering::SeqCst);
        ORIG_DESTROY_LIST.store(ORIG_DESTROY_LIST_PTR as usize, Ordering::SeqCst);
    }
}

unsafe extern "system" fn hooked_get_class_devs_a(
    class_guid: *const Guid,
    enumerator: *const c_char,
    hwnd_parent: usize,
    flags: u32,
) -> usize {
    let original_addr = ORIG_GET_CLASS_DEVS_A.load(Ordering::SeqCst);
    let wants_io4 = has_phantom_hid() && is_hid_guid(class_guid);
    if original_addr == 0 {
        return if wants_io4 { FAKE_DEVICE_INFO_SET } else { 0 };
    }
    let original: SetupDiGetClassDevsAFn = std::mem::transmute(original_addr);
    let handle = original(class_guid, enumerator, hwnd_parent, flags);
    if wants_io4 {
        if handle != 0 && handle != INVALID_HANDLE_VALUE {
            if let Ok(mut sets) = PHANTOM_SETS.lock() {
                sets.insert(handle);
            }
        } else {
            return FAKE_DEVICE_INFO_SET;
        }
    }
    handle
}

unsafe extern "system" fn hooked_get_class_devs(
    class_guid: *const Guid,
    enumerator: *const u16,
    hwnd_parent: usize,
    flags: u32,
) -> usize {
    let original_addr = ORIG_GET_CLASS_DEVS.load(Ordering::SeqCst);
    let wants_io4 = has_phantom_hid() && is_hid_guid(class_guid);
    if original_addr == 0 {
        return if wants_io4 { FAKE_DEVICE_INFO_SET } else { 0 };
    }
    let original: SetupDiGetClassDevsWFn = std::mem::transmute(original_addr);
    let handle = original(class_guid, enumerator, hwnd_parent, flags);
    if wants_io4 {
        if handle != 0 && handle != INVALID_HANDLE_VALUE {
            if let Ok(mut sets) = PHANTOM_SETS.lock() {
                sets.insert(handle);
            }
        } else {
            return FAKE_DEVICE_INFO_SET;
        }
    }
    handle
}

unsafe extern "system" fn hooked_enum_device_interfaces(
    device_info_set: usize,
    device_info_data: *mut SpDevinfoData,
    interface_class_guid: *const Guid,
    member_index: u32,
    device_interface_data: *mut SpDeviceInterfaceData,
) -> i32 {
    if device_info_set == FAKE_DEVICE_INFO_SET {
        if member_index != 0
            || !is_hid_guid(interface_class_guid)
            || !has_phantom_hid()
            || device_interface_data.is_null()
        {
            SetLastError(ERROR_NO_MORE_ITEMS);
            return 0;
        }

        (*device_interface_data).cb_size = std::mem::size_of::<SpDeviceInterfaceData>() as u32;
        (*device_interface_data).interface_class_guid = GUID_DEVINTERFACE_HID;
        (*device_interface_data).flags = 1;
        (*device_interface_data).reserved = FAKE_DEVICE_INFO_SET;
        return 1;
    }

    let original_addr = ORIG_ENUM_INTERFACES.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }

    let original: SetupDiEnumDeviceInterfacesFn = std::mem::transmute(original_addr);
    let registered = is_hid_guid(interface_class_guid)
        && PHANTOM_SETS
            .lock()
            .is_ok_and(|sets| sets.contains(&device_info_set));
    if !registered {
        return original(
            device_info_set,
            device_info_data,
            interface_class_guid,
            member_index,
            device_interface_data,
        );
    }
    if member_index > 0 {
        return original(
            device_info_set,
            device_info_data,
            interface_class_guid,
            member_index - 1,
            device_interface_data,
        );
    }
    if device_interface_data.is_null() {
        return original(
            device_info_set,
            device_info_data,
            interface_class_guid,
            member_index,
            device_interface_data,
        );
    }
    (*device_interface_data).cb_size = std::mem::size_of::<SpDeviceInterfaceData>() as u32;
    (*device_interface_data).interface_class_guid = GUID_DEVINTERFACE_HID;
    (*device_interface_data).flags = 1;
    (*device_interface_data).reserved = FAKE_DEVICE_INFO_SET;
    1
}

unsafe extern "system" fn hooked_get_device_interface_detail(
    device_info_set: usize,
    device_interface_data: *mut SpDeviceInterfaceData,
    device_interface_detail_data: *mut c_void,
    device_interface_detail_data_size: u32,
    required_size: *mut u32,
    device_info_data: *mut SpDevinfoData,
) -> i32 {
    if is_phantom_detail(device_info_set, device_interface_data) {
        let Some(path) = phantom_hid_path() else {
            SetLastError(ERROR_NO_MORE_ITEMS);
            return 0;
        };
        let needed = SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE + (path.len() as u32 * 2);
        if !required_size.is_null() {
            *required_size = needed;
        }
        if device_interface_detail_data.is_null() || device_interface_detail_data_size < needed {
            SetLastError(ERROR_INSUFFICIENT_BUFFER);
            return 0;
        }

        *(device_interface_detail_data as *mut u32) = SP_DEVICE_INTERFACE_DETAIL_DATA_W_CB_SIZE;
        let path_ptr = (device_interface_detail_data as *mut u8)
            .add(SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE as usize)
            as *mut u16;
        ptr::copy_nonoverlapping(path.as_ptr(), path_ptr, path.len());

        if !device_info_data.is_null() {
            (*device_info_data).cb_size = std::mem::size_of::<SpDevinfoData>() as u32;
            (*device_info_data).class_guid = GUID_DEVINTERFACE_HID;
            (*device_info_data).dev_inst = 0;
            (*device_info_data).reserved = FAKE_DEVICE_INFO_SET;
        }
        return 1;
    }

    let original_addr = ORIG_GET_DETAIL.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }

    let original: SetupDiGetDeviceInterfaceDetailWFn = std::mem::transmute(original_addr);
    original(
        device_info_set,
        device_interface_data,
        device_interface_detail_data,
        device_interface_detail_data_size,
        required_size,
        device_info_data,
    )
}

unsafe extern "system" fn hooked_get_device_interface_detail_a(
    device_info_set: usize,
    device_interface_data: *mut SpDeviceInterfaceData,
    device_interface_detail_data: *mut c_void,
    device_interface_detail_data_size: u32,
    required_size: *mut u32,
    device_info_data: *mut SpDevinfoData,
) -> i32 {
    if is_phantom_detail(device_info_set, device_interface_data) {
        let Some(wide_path) = phantom_hid_path() else {
            SetLastError(ERROR_NO_MORE_ITEMS);
            return 0;
        };
        let path = String::from_utf16_lossy(&wide_path[..wide_path.len().saturating_sub(1)]);
        let mut path = path.into_bytes();
        path.push(0);
        let needed = SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE + path.len() as u32;
        if !required_size.is_null() {
            *required_size = needed;
        }
        if device_interface_detail_data.is_null() || device_interface_detail_data_size < needed {
            SetLastError(ERROR_INSUFFICIENT_BUFFER);
            return 0;
        }

        *(device_interface_detail_data as *mut u32) = if cfg!(target_pointer_width = "64") {
            8
        } else {
            5
        };
        let path_ptr = (device_interface_detail_data as *mut u8)
            .add(SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE as usize);
        ptr::copy_nonoverlapping(path.as_ptr(), path_ptr, path.len());
        if !device_info_data.is_null() {
            (*device_info_data).cb_size = std::mem::size_of::<SpDevinfoData>() as u32;
            (*device_info_data).class_guid = GUID_DEVINTERFACE_HID;
            (*device_info_data).dev_inst = 0;
            (*device_info_data).reserved = FAKE_DEVICE_INFO_SET;
        }
        return 1;
    }

    let original_addr = ORIG_GET_DETAIL_A.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: SetupDiGetDeviceInterfaceDetailAFn = std::mem::transmute(original_addr);
    original(
        device_info_set,
        device_interface_data,
        device_interface_detail_data,
        device_interface_detail_data_size,
        required_size,
        device_info_data,
    )
}

unsafe extern "system" fn hooked_destroy_device_info_list(device_info_set: usize) -> i32 {
    if device_info_set == FAKE_DEVICE_INFO_SET {
        return 1;
    }
    if let Ok(mut sets) = PHANTOM_SETS.lock() {
        sets.remove(&device_info_set);
    }

    let original_addr = ORIG_DESTROY_LIST.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }

    let original: SetupDiDestroyDeviceInfoListFn = std::mem::transmute(original_addr);
    original(device_info_set)
}

unsafe fn is_hid_guid(guid: *const Guid) -> bool {
    !guid.is_null() && guid_eq(&*guid, &GUID_DEVINTERFACE_HID)
}

fn has_phantom_hid() -> bool {
    PHANTOM_HID_PATH
        .lock()
        .is_ok_and(|registered| registered.is_some())
}

fn phantom_hid_path() -> Option<Vec<u16>> {
    PHANTOM_HID_PATH
        .lock()
        .ok()
        .and_then(|registered| registered.clone())
}

unsafe fn is_phantom_detail(
    device_info_set: usize,
    device_interface_data: *mut SpDeviceInterfaceData,
) -> bool {
    device_info_set == FAKE_DEVICE_INFO_SET
        || (!device_interface_data.is_null()
            && (*device_interface_data).reserved == FAKE_DEVICE_INFO_SET)
}

fn guid_eq(left: &Guid, right: &Guid) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}
