use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, LoadLibraryA};

use applechu::amdaemon::EpayConfig;
use applechu::iohook::hook_table::{
    hook_table_apply, hook_table_apply_ordinals, null_module, HookSymbol, OrdinalHookSymbol,
};
use applechu::iohook::proc_addr;
use applechu::platform::reg_hook::{self, RegValue, HKEY_LOCAL_MACHINE};
use applechu::util::api::Api;

type GetVersionFn = unsafe extern "system" fn() -> u64;
type GetInstanceFn = unsafe extern "system" fn(u64) -> *mut ThincaMain;

static ORIG_GET_VERSION: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_INSTANCE: AtomicUsize = AtomicUsize::new(0);
static mut ORIG_GET_VERSION_PTR: *const () = ptr::null();
static mut ORIG_GET_INSTANCE_PTR: *const () = ptr::null();
static STUB: OnceLock<usize> = OnceLock::new();

#[repr(C, packed)]
struct ThincaImpl {
    unk0: usize,
    unk8: usize,
    initialize: usize,
    dispose: usize,
    set_resource: usize,
    set_payment_log: usize,
    set_client_log: usize,
    set_client_config: usize,
    unk40: usize,
    unk48: usize,
    set_client_certificate: usize,
    set_terminal_serial: usize,
    set_goods_code: usize,
    unk68: u64,
    set_event_interface: usize,
    gap78: [u64; 7],
    check_deal: usize,
    gap_b8: [u64; 41],
    cancel_request: usize,
    select_button: usize,
    gap210: [u64; 2],
    unk220: usize,
    unk228: usize,
}

#[repr(C, packed)]
struct ThincaMain {
    impl1: *mut ThincaImpl,
    rest: [usize; 97],
}

const _: () = assert!(std::mem::size_of::<ThincaImpl>() == 0x230);
const _: () = assert!(std::mem::size_of::<ThincaMain>() == 0x310);

#[applechu_macros::config_section(stage = Platform, order = 95)]
pub fn init(api: &Api, config: &EpayConfig) {
    register_values();
    preload_dependencies(api);
    if !config.hook {
        api.log_info("E-pay registry compatibility enabled");
        return;
    }

    let _ = STUB.set(create_stub() as usize);
    unsafe {
        let symbols = [
            HookSymbol {
                name: "ThincaPaymentGetVersion",
                patch: hooked_get_version as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_VERSION_PTR),
            },
            HookSymbol {
                name: "__imp_ThincaPaymentGetInstance",
                patch: hooked_get_instance as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_INSTANCE_PTR),
            },
            HookSymbol {
                name: "ThincaPaymentGetInstance",
                patch: hooked_get_instance as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_INSTANCE_PTR),
            },
        ];
        proc_addr::push("ThincaPayment.dll", &symbols, sync_originals);
        proc_addr::push_get_proc_ordinal_override(thinca_ordinal_override);
        let mut patched = hook_table_apply(null_module(), "ThincaPayment.dll", &symbols);
        let ordinal_symbols = [
            OrdinalHookSymbol {
                ordinal: 1,
                patch: hooked_get_version as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_VERSION_PTR),
            },
            OrdinalHookSymbol {
                ordinal: 2,
                patch: hooked_get_instance as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_INSTANCE_PTR),
            },
        ];
        patched += hook_table_apply_ordinals(null_module(), "ThincaPayment.dll", &ordinal_symbols);
        sync_originals();
        api.log_info(&format!(
            "E-pay compatibility ready with {patched} patched entries"
        ));
    }
}

fn thinca_ordinal_override(module: usize, ordinal: u16) -> Option<*const ()> {
    let thinca = unsafe { GetModuleHandleA(c"ThincaPayment.dll".as_ptr().cast()) };
    if thinca.is_null() || thinca as usize != module {
        return None;
    }
    match ordinal {
        1 => Some(hooked_get_version as *const ()),
        2 => Some(hooked_get_instance as *const ()),
        _ => None,
    }
}

fn register_values() {
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\TFPaymentService\\ThincaRwAdapter",
        vec![RegValue::string(
            "TfpsAimeRwAdapter",
            "aime_rw_adapterMD.dll",
        )],
    );
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\TFPaymentService\\ThincaTcapClient",
        vec![
            RegValue::string("CaLocation", "ca.pem"),
            RegValue::string("ThincaTcapClientPath", "thincatcapclient.dll"),
            RegValue::dword("ClientNetworkTimeout", 20_000),
        ],
    );
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\TFPaymentService\\ThincaTcapClient\\URL0",
        vec![
            RegValue::string("Pattern", ".*\\.jsp"),
            RegValue::dword("ClientNetworkTimeout", 5_000),
        ],
    );
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SOFTWARE\\TFPaymentService\\ThincaTcapClient\\URL1",
        vec![
            RegValue::string("Pattern", ".*(closing|remove).*"),
            RegValue::dword("ClientNetworkTimeout", 60_000),
        ],
    );
}

fn preload_dependencies(api: &Api) {
    for (name, apply_dns) in [
        (b"thincahttpclient.dll\0".as_slice(), true),
        (b"ThincaPayment.dll\0".as_slice(), false),
        (b"thincatcapclient.dll\0".as_slice(), false),
    ] {
        let module = unsafe { LoadLibraryA(name.as_ptr()) };
        if module.is_null() {
            api.log_warn(&format!(
                "Failed to load E-pay dependency: {}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            ));
            continue;
        }

        let path_patched = unsafe { applechu::platform::path_hook::apply_hooks(module) };
        let dns_patched = if apply_dns {
            unsafe { crate::dns::apply_hooks(module) }
        } else {
            0
        };
        if path_patched + dns_patched > 0 {
            api.log_info(&format!(
                "E-pay dependency attached: {}",
                String::from_utf8_lossy(&name[..name.len() - 1])
            ));
        }
    }
}

fn create_stub() -> *mut ThincaMain {
    let implementation = Box::new(ThincaImpl {
        unk0: 0,
        unk8: thinca_unk8 as *const () as usize,
        initialize: thinca_initialize as *const () as usize,
        dispose: thinca_no_args as *const () as usize,
        set_resource: thinca_string as *const () as usize,
        set_payment_log: thinca_payment_log as *const () as usize,
        set_client_log: thinca_client_log as *const () as usize,
        set_client_config: thinca_client_config as *const () as usize,
        unk40: 0,
        unk48: 0,
        set_client_certificate: thinca_string_value as *const () as usize,
        set_terminal_serial: thinca_string as *const () as usize,
        set_goods_code: thinca_string as *const () as usize,
        unk68: 0,
        set_event_interface: thinca_pointer as *const () as usize,
        gap78: [0; 7],
        check_deal: thinca_pointer as *const () as usize,
        gap_b8: [0; 41],
        cancel_request: thinca_no_args as *const () as usize,
        select_button: thinca_no_args as *const () as usize,
        gap210: [0; 2],
        unk220: thinca_value as *const () as usize,
        unk228: thinca_value as *const () as usize,
    });
    Box::into_raw(Box::new(ThincaMain {
        impl1: Box::into_raw(implementation),
        rest: [0; 97],
    }))
}

fn sync_originals() {
    ORIG_GET_VERSION.store(unsafe { ORIG_GET_VERSION_PTR as usize }, Ordering::Release);
    ORIG_GET_INSTANCE.store(unsafe { ORIG_GET_INSTANCE_PTR as usize }, Ordering::Release);
}

unsafe extern "system" fn hooked_get_version() -> u64 {
    0x0104_0B00
}

unsafe extern "system" fn hooked_get_instance(_version: u64) -> *mut ThincaMain {
    STUB.get()
        .copied()
        .map_or(ptr::null_mut(), |stub| stub as *mut ThincaMain)
}

unsafe extern "system" fn thinca_unk8(_this: *mut ThincaImpl) {}
unsafe extern "system" fn thinca_initialize(_this: *mut ThincaImpl, _value: u64) -> u64 {
    0
}
unsafe extern "system" fn thinca_no_args(_this: *mut ThincaImpl) -> u64 {
    0
}
unsafe extern "system" fn thinca_string(_this: *mut ThincaImpl, _value: *mut c_char) -> u64 {
    0
}
unsafe extern "system" fn thinca_string_value(
    _this: *mut ThincaImpl,
    _value: *mut c_char,
    _number: u64,
) -> u64 {
    0
}
unsafe extern "system" fn thinca_payment_log(
    _this: *mut ThincaImpl,
    _value: u64,
    _log: *mut c_char,
    _value2: u64,
    _limit: *const c_char,
) -> u64 {
    0
}
unsafe extern "system" fn thinca_client_log(
    _this: *mut ThincaImpl,
    _value: u64,
    _log: *mut c_char,
) -> u64 {
    0
}
unsafe extern "system" fn thinca_client_config(
    _this: *mut ThincaImpl,
    _log: *mut c_char,
    _value: u64,
) -> u64 {
    0
}
unsafe extern "system" fn thinca_pointer(_this: *mut ThincaImpl, _value: *mut c_void) -> u64 {
    0
}
unsafe extern "system" fn thinca_value(_this: *mut ThincaImpl, _value: u64) -> u64 {
    0
}

#[allow(dead_code)]
unsafe fn original_version() -> Option<GetVersionFn> {
    let address = ORIG_GET_VERSION.load(Ordering::Acquire);
    (address != 0).then(|| std::mem::transmute(address))
}

#[allow(dead_code)]
unsafe fn original_instance() -> Option<GetInstanceFn> {
    let address = ORIG_GET_INSTANCE.load(Ordering::Acquire);
    (address != 0).then(|| std::mem::transmute(address))
}
