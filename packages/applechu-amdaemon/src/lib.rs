#![allow(dead_code, non_snake_case)]

#[cfg(not(target_arch = "x86_64"))]
compile_error!(
    "applechu-amdaemon 必须使用 x86_64-pc-windows-msvc 编译；游戏侧 winhttp.dll 才使用 i686 目标"
);

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

extern crate winhttp as applechu;

// 属性宏生成的注册代码使用当前 crate 的统一路径；这些模块只是公共基础设施的
// 公开转发，不包含 AM Daemon 的专用实现
pub use applechu::{config, iohook, module_registry, platform, util};

mod amvideo_loader;
mod command_line;
mod console;
mod crash;
mod dns;
mod epay;
mod ewf;
mod exit_trace;
mod hwmon;
mod hwreset;
mod netenv;
mod nusec;
mod openssl;
mod startup;

use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;

const DLL_PROCESS_ATTACH: u32 = 1;
static MODULE: AtomicUsize = AtomicUsize::new(0);

core::arch::global_asm!(
    ".globl DllMain",
    "DllMain:",
    "jmp {entry}",
    entry = sym dll_main,
);

pub(crate) fn module_handle() -> usize {
    MODULE.load(Ordering::Acquire)
}

unsafe extern "system" fn dll_main(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        MODULE.store(module as usize, Ordering::Release);
        DisableThreadLibraryCalls(module);
        startup::install();
    }

    TRUE
}
