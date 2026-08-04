use std::ffi::{c_char, CStr};
use std::ptr;

use windows_sys::Win32::Foundation::{SetLastError, HANDLE, HMODULE, TRUE};

use applechu::iohook::hook_table::{self, HookSymbol};

type GetModuleHandleAFn = unsafe extern "system" fn(*const c_char) -> HMODULE;
type GetModuleHandleWFn = unsafe extern "system" fn(*const u16) -> HMODULE;
type GetProcAddressFn = unsafe extern "system" fn(HMODULE, *const u8) -> *const ();
type LoadLibraryAFn = unsafe extern "system" fn(*const c_char) -> HMODULE;
type LoadLibraryWFn = unsafe extern "system" fn(*const u16) -> HMODULE;
type LoadLibraryExAFn = unsafe extern "system" fn(*const c_char, HANDLE, u32) -> HMODULE;
type LoadLibraryExWFn = unsafe extern "system" fn(*const u16, HANDLE, u32) -> HMODULE;
type FreeLibraryFn = unsafe extern "system" fn(HMODULE) -> i32;

static mut ORIG_GET_MODULE_HANDLE_A: *const () = ptr::null();
static mut ORIG_GET_MODULE_HANDLE_W: *const () = ptr::null();
static mut ORIG_GET_PROC_ADDRESS: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_A: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_W: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_EX_A: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_EX_W: *const () = ptr::null();
static mut ORIG_FREE_LIBRARY: *const () = ptr::null();

/// `$amvideo` 是由当前代理提供的常驻伪模块
/// 共享 proc_addr 负责其余代理链的转发，这里确保伪模块句柄来自 winmm
pub(crate) fn install() {
    let symbols = [
        HookSymbol {
            name: "GetModuleHandleA",
            patch: hooked_get_module_handle_a as *const (),
            original: ptr::addr_of_mut!(ORIG_GET_MODULE_HANDLE_A),
        },
        HookSymbol {
            name: "GetModuleHandleW",
            patch: hooked_get_module_handle_w as *const (),
            original: ptr::addr_of_mut!(ORIG_GET_MODULE_HANDLE_W),
        },
        HookSymbol {
            name: "GetProcAddress",
            patch: hooked_get_proc_address as *const (),
            original: ptr::addr_of_mut!(ORIG_GET_PROC_ADDRESS),
        },
        HookSymbol {
            name: "LoadLibraryA",
            patch: hooked_load_library_a as *const (),
            original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_A),
        },
        HookSymbol {
            name: "LoadLibraryW",
            patch: hooked_load_library_w as *const (),
            original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_W),
        },
        HookSymbol {
            name: "LoadLibraryExA",
            patch: hooked_load_library_ex_a as *const (),
            original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_EX_A),
        },
        HookSymbol {
            name: "LoadLibraryExW",
            patch: hooked_load_library_ex_w as *const (),
            original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_EX_W),
        },
        HookSymbol {
            name: "FreeLibrary",
            patch: hooked_free_library as *const (),
            original: ptr::addr_of_mut!(ORIG_FREE_LIBRARY),
        },
    ];
    let patched = unsafe {
        hook_table::hook_table_apply(hook_table::null_module(), "kernel32.dll", &symbols)
    };
    crate::console::info(&format!(
        "AMVideo loader compatibility installed for {patched} entries"
    ));
}

fn redirected_module() -> HMODULE {
    applechu::util::win32::handle_from_value(crate::module_handle())
}

unsafe extern "system" fn hooked_get_module_handle_a(name: *const c_char) -> HMODULE {
    if is_amvideo_a(name) {
        redirected_module()
    } else {
        original_get_module_handle_a(name)
    }
}

unsafe extern "system" fn hooked_get_module_handle_w(name: *const u16) -> HMODULE {
    if is_amvideo_w(name) {
        redirected_module()
    } else {
        original_get_module_handle_w(name)
    }
}

unsafe extern "system" fn hooked_get_proc_address(module: HMODULE, name: *const u8) -> *const () {
    if module.addr() == crate::module_handle() {
        if (name as usize) <= 0xFFFF {
            return match name as usize as u16 {
                1 => applechu::platform::amvideo::am_dll_video_open as *const (),
                2 => applechu::platform::amvideo::am_dll_video_close as *const (),
                3 => applechu::platform::amvideo::am_dll_video_set_resolution as *const (),
                4 => applechu::platform::amvideo::am_dll_video_get_vbios_version as *const (),
                _ => original_get_proc_address(module, name),
            };
        }
        if !name.is_null() {
            let proc_name = CStr::from_ptr(name.cast()).to_bytes();
            return match proc_name {
                b"amDllVideoOpen" => applechu::platform::amvideo::am_dll_video_open as *const (),
                b"amDllVideoClose" => applechu::platform::amvideo::am_dll_video_close as *const (),
                b"amDllVideoSetResolution" => {
                    applechu::platform::amvideo::am_dll_video_set_resolution as *const ()
                }
                b"amDllVideoGetVBiosVersion" => {
                    applechu::platform::amvideo::am_dll_video_get_vbios_version as *const ()
                }
                _ => original_get_proc_address(module, name),
            };
        }
    }
    original_get_proc_address(module, name)
}

unsafe extern "system" fn hooked_load_library_a(name: *const c_char) -> HMODULE {
    if is_amvideo_a(name) {
        redirected_module()
    } else {
        original_load_library_a(name)
    }
}

unsafe extern "system" fn hooked_load_library_w(name: *const u16) -> HMODULE {
    if is_amvideo_w(name) {
        redirected_module()
    } else {
        original_load_library_w(name)
    }
}

unsafe extern "system" fn hooked_load_library_ex_a(
    name: *const c_char,
    file: HANDLE,
    flags: u32,
) -> HMODULE {
    if is_amvideo_a(name) {
        redirected_module()
    } else {
        original_load_library_ex_a(name, file, flags)
    }
}

unsafe extern "system" fn hooked_load_library_ex_w(
    name: *const u16,
    file: HANDLE,
    flags: u32,
) -> HMODULE {
    if is_amvideo_w(name) {
        redirected_module()
    } else {
        original_load_library_ex_w(name, file, flags)
    }
}

unsafe extern "system" fn hooked_free_library(module: HMODULE) -> i32 {
    if !module.is_null() && module.addr() == crate::module_handle() {
        SetLastError(0);
        TRUE
    } else {
        original_free_library(module)
    }
}

unsafe fn original_get_module_handle_a(name: *const c_char) -> HMODULE {
    if ORIG_GET_MODULE_HANDLE_A.is_null() {
        ptr::null_mut()
    } else {
        std::mem::transmute::<*const (), GetModuleHandleAFn>(ORIG_GET_MODULE_HANDLE_A)(name)
    }
}

unsafe fn original_get_module_handle_w(name: *const u16) -> HMODULE {
    if ORIG_GET_MODULE_HANDLE_W.is_null() {
        ptr::null_mut()
    } else {
        std::mem::transmute::<*const (), GetModuleHandleWFn>(ORIG_GET_MODULE_HANDLE_W)(name)
    }
}

unsafe fn original_get_proc_address(module: HMODULE, name: *const u8) -> *const () {
    if ORIG_GET_PROC_ADDRESS.is_null() {
        ptr::null()
    } else {
        std::mem::transmute::<*const (), GetProcAddressFn>(ORIG_GET_PROC_ADDRESS)(module, name)
    }
}

unsafe fn original_load_library_a(name: *const c_char) -> HMODULE {
    if ORIG_LOAD_LIBRARY_A.is_null() {
        ptr::null_mut()
    } else {
        std::mem::transmute::<*const (), LoadLibraryAFn>(ORIG_LOAD_LIBRARY_A)(name)
    }
}

unsafe fn original_load_library_w(name: *const u16) -> HMODULE {
    if ORIG_LOAD_LIBRARY_W.is_null() {
        ptr::null_mut()
    } else {
        std::mem::transmute::<*const (), LoadLibraryWFn>(ORIG_LOAD_LIBRARY_W)(name)
    }
}

unsafe fn original_load_library_ex_a(name: *const c_char, file: HANDLE, flags: u32) -> HMODULE {
    if ORIG_LOAD_LIBRARY_EX_A.is_null() {
        ptr::null_mut()
    } else {
        std::mem::transmute::<*const (), LoadLibraryExAFn>(ORIG_LOAD_LIBRARY_EX_A)(
            name, file, flags,
        )
    }
}

unsafe fn original_load_library_ex_w(name: *const u16, file: HANDLE, flags: u32) -> HMODULE {
    if ORIG_LOAD_LIBRARY_EX_W.is_null() {
        ptr::null_mut()
    } else {
        std::mem::transmute::<*const (), LoadLibraryExWFn>(ORIG_LOAD_LIBRARY_EX_W)(
            name, file, flags,
        )
    }
}

unsafe fn original_free_library(module: HMODULE) -> i32 {
    if ORIG_FREE_LIBRARY.is_null() {
        0
    } else {
        std::mem::transmute::<*const (), FreeLibraryFn>(ORIG_FREE_LIBRARY)(module)
    }
}

unsafe fn is_amvideo_a(name: *const c_char) -> bool {
    if name.is_null() {
        return false;
    }
    CStr::from_ptr(name)
        .to_bytes()
        .eq_ignore_ascii_case(b"$amvideo")
}

unsafe fn is_amvideo_w(name: *const u16) -> bool {
    if name.is_null() {
        return false;
    }
    let mut length = 0usize;
    while *name.add(length) != 0 {
        length += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(name, length))
        .eq_ignore_ascii_case("$amvideo")
}
