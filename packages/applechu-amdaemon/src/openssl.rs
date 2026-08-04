use std::ffi::{c_char, CStr};
use std::sync::atomic::{AtomicUsize, Ordering};

use applechu::amdaemon::OpenSslConfig;
use applechu::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use applechu::iohook::proc_addr;
use applechu::util::api::Api;

type GetenvFn = unsafe extern "C" fn(*const c_char) -> *mut c_char;

static ORIG_GETENV: AtomicUsize = AtomicUsize::new(0);
static mut ORIG_GETENV_PTR: *const () = std::ptr::null();
static OPENSSL_IA32CAP: [u8; 12] = *b"~0x20000000\0";

#[applechu_macros::config_section(stage = Platform, order = 110)]
pub fn init(api: &Api, config: &OpenSslConfig) {
    if !config.force_legacy_sha && !has_intel_sha_extensions() {
        return;
    }
    unsafe {
        let symbols = [HookSymbol {
            name: "getenv",
            patch: hooked_getenv as *const (),
            original: std::ptr::addr_of_mut!(ORIG_GETENV_PTR),
        }];
        proc_addr::push("msvcr110.dll", &symbols, sync_original);
        let patched = hook_table_apply(null_module(), "msvcr110.dll", &symbols);
        sync_original();
        if patched == 0 && ORIG_GETENV.load(Ordering::Acquire) == 0 {
            return api.log_warn("OpenSSL compatibility could not find msvcr110 getenv");
        }
    }
    api.log_info("OpenSSL SHA compatibility ready");
}

fn has_intel_sha_extensions() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        let vendor = std::arch::x86_64::__cpuid(0);
        if [vendor.ebx, vendor.edx, vendor.ecx] != [0x756e_6547, 0x4965_6e69, 0x6c65_746e] {
            return false;
        }
        vendor.eax >= 7 && std::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 29) != 0
    }
    #[cfg(target_arch = "x86")]
    {
        let vendor = std::arch::x86::__cpuid(0);
        if [vendor.ebx, vendor.edx, vendor.ecx] != [0x756e_6547, 0x4965_6e69, 0x6c65_746e] {
            return false;
        }
        vendor.eax >= 7 && std::arch::x86::__cpuid_count(7, 0).ebx & (1 << 29) != 0
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        false
    }
}

fn sync_original() {
    ORIG_GETENV.store(unsafe { ORIG_GETENV_PTR as usize }, Ordering::Release);
}

unsafe extern "C" fn hooked_getenv(name: *const c_char) -> *mut c_char {
    if !name.is_null() && CStr::from_ptr(name).to_bytes() == b"OPENSSL_ia32cap" {
        return OPENSSL_IA32CAP.as_ptr().cast_mut().cast();
    }
    let original = ORIG_GETENV.load(Ordering::Acquire);
    if original == 0 {
        return std::ptr::null_mut();
    }
    std::mem::transmute::<usize, GetenvFn>(original)(name)
}
