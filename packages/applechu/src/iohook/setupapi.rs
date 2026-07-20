use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

use crate::iohook::{hook_table::HookSymbol, proc_addr};
use crate::util::api::Api;

const DIGCF_DEVICEINTERFACE: u32 = 0x0000_0010;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const FAKE_DEVICE_INFO_SET: usize = 0x4348_4944;
const PHANTOM_PATH: [u16; 14] = [
    b'$' as u16,
    b'i' as u16,
    b'o' as u16,
    b'4' as u16,
    b'\\' as u16,
    b'v' as u16,
    b'i' as u16,
    b'd' as u16,
    b'_' as u16,
    b'0' as u16,
    b'c' as u16,
    b'a' as u16,
    b'3' as u16,
    0,
];
const SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE: u32 = 4;

type SetupDiGetClassDevsWFn =
    unsafe extern "system" fn(*const Guid, *const u16, usize, u32) -> usize;
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
type SetupDiDestroyDeviceInfoListFn = unsafe extern "system" fn(usize) -> i32;

static ORIG_GET_CLASS_DEVS: AtomicUsize = AtomicUsize::new(0);
static ORIG_ENUM_INTERFACES: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_DETAIL: AtomicUsize = AtomicUsize::new(0);
static ORIG_DESTROY_LIST: AtomicUsize = AtomicUsize::new(0);
static PHANTOM_SETS: Lazy<Mutex<HashMap<usize, Option<u32>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static mut ORIG_GET_CLASS_DEVS_PTR: *const () = std::ptr::null();
static mut ORIG_ENUM_INTERFACES_PTR: *const () = std::ptr::null();
static mut ORIG_GET_DETAIL_PTR: *const () = std::ptr::null();
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
    fn GetModuleHandleA(name: *const u8) -> usize;
}

pub fn init(api: &Api) {
    unsafe {
        let symbols = [
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
        let loaded = GetModuleHandleA(b"setupapi.dll\0".as_ptr()) != 0;
        api.log_info(&format!(
            "iohook: setupapi hook_table_apply patched={patched}, setupapi.dll loaded={loaded}"
        ));
        if patched > 0 {
            api.log_info(&format!("iohook: setupapi hooks installed ({patched})"));
        }
    }
}

fn sync_originals() {
    // SAFETY: 原函数槽由 hook 安装过程写入，随后通过原子变量发布给并发调用者。
    unsafe {
        ORIG_GET_CLASS_DEVS.store(ORIG_GET_CLASS_DEVS_PTR as usize, Ordering::SeqCst);
        ORIG_ENUM_INTERFACES.store(ORIG_ENUM_INTERFACES_PTR as usize, Ordering::SeqCst);
        ORIG_GET_DETAIL.store(ORIG_GET_DETAIL_PTR as usize, Ordering::SeqCst);
        ORIG_DESTROY_LIST.store(ORIG_DESTROY_LIST_PTR as usize, Ordering::SeqCst);
    }
}

unsafe extern "system" fn hooked_get_class_devs(
    class_guid: *const Guid,
    enumerator: *const u16,
    hwnd_parent: usize,
    flags: u32,
) -> usize {
    let original_addr = ORIG_GET_CLASS_DEVS.load(Ordering::SeqCst);
    let wants_io4 =
        flags & DIGCF_DEVICEINTERFACE != 0 && enumerator.is_null() && is_hid_guid(class_guid);
    if original_addr == 0 {
        return if wants_io4 { FAKE_DEVICE_INFO_SET } else { 0 };
    }
    let original: SetupDiGetClassDevsWFn = std::mem::transmute(original_addr);
    let handle = original(class_guid, enumerator, hwnd_parent, flags);
    if wants_io4 {
        if handle != 0 {
            if let Ok(mut sets) = PHANTOM_SETS.lock() {
                sets.insert(handle, None);
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
            || device_interface_data.is_null()
        {
            SetLastError(ERROR_NO_MORE_ITEMS);
            return 0;
        }

        (*device_interface_data).cb_size = std::mem::size_of::<SpDeviceInterfaceData>() as u32;
        (*device_interface_data).interface_class_guid = GUID_DEVINTERFACE_HID;
        (*device_interface_data).flags = 0;
        (*device_interface_data).reserved = FAKE_DEVICE_INFO_SET;
        return 1;
    }

    let original_addr = ORIG_ENUM_INTERFACES.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }

    let original: SetupDiEnumDeviceInterfacesFn = std::mem::transmute(original_addr);
    let ok = original(
        device_info_set,
        device_info_data,
        interface_class_guid,
        member_index,
        device_interface_data,
    );
    if ok != 0 {
        return ok;
    }
    if !is_hid_guid(interface_class_guid) || device_interface_data.is_null() {
        return 0;
    }
    let Some(phantom_index) = phantom_index_for(device_info_set, member_index) else {
        return 0;
    };
    if member_index != phantom_index {
        SetLastError(ERROR_NO_MORE_ITEMS);
        return 0;
    }
    (*device_interface_data).cb_size = std::mem::size_of::<SpDeviceInterfaceData>() as u32;
    (*device_interface_data).interface_class_guid = GUID_DEVINTERFACE_HID;
    (*device_interface_data).flags = 0;
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
        let needed =
            SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE + (PHANTOM_PATH.len() as u32 * 2);
        if !required_size.is_null() {
            *required_size = needed;
        }
        if device_interface_detail_data.is_null() || device_interface_detail_data_size < needed {
            SetLastError(ERROR_INSUFFICIENT_BUFFER);
            return 0;
        }

        *(device_interface_detail_data as *mut u32) = std::mem::size_of::<u32>() as u32 + 2;
        let path_ptr = (device_interface_detail_data as *mut u8)
            .add(SP_DEVICE_INTERFACE_DETAIL_DATA_W_PREFIX_SIZE as usize)
            as *mut u16;
        ptr::copy_nonoverlapping(PHANTOM_PATH.as_ptr(), path_ptr, PHANTOM_PATH.len());

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
    guid.is_null() || guid_eq(&*guid, &GUID_DEVINTERFACE_HID)
}

fn phantom_index_for(device_info_set: usize, member_index: u32) -> Option<u32> {
    let Ok(mut sets) = PHANTOM_SETS.lock() else {
        return None;
    };
    let index = sets.get_mut(&device_info_set)?;
    Some(*index.get_or_insert(member_index))
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
