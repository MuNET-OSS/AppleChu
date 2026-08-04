use std::ptr;

use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use crate::util::api::Api;

type QueryDosDeviceWFn = unsafe extern "system" fn(*const u16, *mut u16, u32) -> u32;

static mut ORIG_QUERY_DOS_DEVICE_W: *const () = ptr::null();

crate::config_section! {
    pub(crate) struct DvdConfig => DVD_CONFIG_SECTION {
        section: "DVD",
        order: 930,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "DVD 路径模拟",
        fields: {}
    }
}

#[applechu_macros::config_section(stage = PlatformCore, order = 5)]
pub(crate) fn init(api: &Api, _config: &DvdConfig) {
    unsafe {
        let symbols = [HookSymbol {
            name: "QueryDosDeviceW",
            patch: hooked_query_dos_device_w as *const (),
            original: ptr::addr_of_mut!(ORIG_QUERY_DOS_DEVICE_W),
        }];
        let patched = hook_table_apply(null_module(), "kernel32.dll", &symbols);
        api.log_info(&format!(
            "DVD path compatibility ready with {patched} patched entries"
        ));
    }
}

unsafe extern "system" fn hooked_query_dos_device_w(
    device_name: *const u16,
    target_path: *mut u16,
    max_chars: u32,
) -> u32 {
    let original: QueryDosDeviceWFn = if ORIG_QUERY_DOS_DEVICE_W.is_null() {
        windows_sys::Win32::Storage::FileSystem::QueryDosDeviceW
    } else {
        std::mem::transmute(ORIG_QUERY_DOS_DEVICE_W)
    };
    let result = original(device_name, target_path, max_chars);
    if result == 0 || target_path.is_null() {
        return result;
    }

    let target = std::slice::from_raw_parts(target_path, result as usize);
    let cdrom = [
        b'C' as u16,
        b'd' as u16,
        b'R' as u16,
        b'o' as u16,
        b'm' as u16,
    ];
    if target.windows(cdrom.len()).any(|part| part == cdrom) {
        return 0;
    }
    result
}
