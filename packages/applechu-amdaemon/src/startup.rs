use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const ENTRY_JUMP_LEN: usize = 14;
const STUB_CAPACITY: usize = 96;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static ENTRY: AtomicPtr<u8> = AtomicPtr::new(ptr::null_mut());
static ORIGINAL: OnceLock<[u8; ENTRY_JUMP_LEN]> = OnceLock::new();

/// winmm 是由 AM Daemon 主线程自然加载的，不能照搬注入器对挂起线程的
/// CONTEXT 劫持。这里仅在 DllMain 中改写 EXE 入口，实际初始化在离开加载器锁后执行
pub(crate) fn install() {
    if INSTALLED.swap(true, Ordering::AcqRel) {
        return;
    }

    unsafe {
        let executable = GetModuleHandleW(ptr::null());
        if executable.is_null() {
            install_failed("GetModuleHandleW(NULL) failed");
            return;
        }
        let Some(entry) = entry_point(executable as usize).map(|address| address as *mut u8) else {
            install_failed("invalid executable entry point");
            return;
        };

        let mut original = [0; ENTRY_JUMP_LEN];
        // SAFETY: entry 指向当前进程已加载 EXE 的可执行入口，长度由固定 x64 跳板协议限定
        ptr::copy_nonoverlapping(entry, original.as_mut_ptr(), ENTRY_JUMP_LEN);
        let _ = ORIGINAL.set(original);
        let stub = build_stub(entry);
        if stub.is_null() {
            install_failed("failed to allocate entry stub");
            return;
        }
        if !write_absolute_jump(entry, stub) {
            install_failed("failed to patch executable entry point");
            return;
        }

        // 当前仍在同一个主线程的 DllMain 中，返回前不会执行到 EXE 入口
        ENTRY.store(entry, Ordering::Release);
    }
}

fn install_failed(message: &str) {
    INSTALLED.store(false, Ordering::Release);
    crate::console::error(&format!("Failed to install the startup entry: {message}"));
}

unsafe fn build_stub(entry: *mut u8) -> *mut u8 {
    let stub = VirtualAlloc(
        ptr::null(),
        STUB_CAPACITY,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_EXECUTE_READWRITE,
    ) as *mut u8;
    if stub.is_null() {
        return ptr::null_mut();
    }

    let mut code = Vec::with_capacity(STUB_CAPACITY);
    code.extend_from_slice(&[
        0x9C, // pushfq
        0x50, // push rax
        0x51, // push rcx
        0x52, // push rdx
        0x41, 0x50, // push r8
        0x41, 0x51, // push r9
        0x41, 0x52, // push r10
        0x41, 0x53, // push r11
        0x48, 0x83, 0xEC, 0x28, // 为 x64 调用约定预留 shadow space 并对齐栈
        0x48, 0xB8, // mov rax, bootstrap
    ]);
    code.extend_from_slice(&(bootstrap as *const () as u64).to_le_bytes());
    code.extend_from_slice(&[
        0xFF, 0xD0, // call rax
        0x48, 0x83, 0xC4, 0x28, // 恢复栈
        0x41, 0x5B, // pop r11
        0x41, 0x5A, // pop r10
        0x41, 0x59, // pop r9
        0x41, 0x58, // pop r8
        0x5A, // pop rdx
        0x59, // pop rcx
        0x58, // pop rax
        0x9D, // popfq
        0xFF, 0x25, 0x00, 0x00, 0x00, 0x00, // jmp qword ptr [rip]
    ]);
    code.extend_from_slice(&(entry as u64).to_le_bytes());
    debug_assert!(code.len() <= STUB_CAPACITY);

    ptr::copy_nonoverlapping(code.as_ptr(), stub, code.len());
    flush(stub.cast(), code.len());
    stub
}

unsafe fn write_absolute_jump(entry: *mut u8, target: *mut u8) -> bool {
    let mut jump = [0u8; ENTRY_JUMP_LEN];
    jump[..6].copy_from_slice(&[0xFF, 0x25, 0x00, 0x00, 0x00, 0x00]);
    jump[6..].copy_from_slice(&(target as u64).to_le_bytes());

    let mut old_protect = 0;
    if VirtualProtect(
        entry.cast(),
        ENTRY_JUMP_LEN,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return false;
    }
    ptr::copy_nonoverlapping(jump.as_ptr(), entry, jump.len());
    let mut ignored = 0;
    let _ = VirtualProtect(entry.cast(), ENTRY_JUMP_LEN, old_protect, &mut ignored);
    flush(entry.cast(), ENTRY_JUMP_LEN);
    true
}

unsafe extern "system" fn bootstrap() {
    // 必须先恢复入口；初始化完成后跳板会从原始入口重新开始执行
    restore_entry();

    let base_dir = executable_base_dir();
    let console_ready = crate::console::initialize(&base_dir);
    if !console_ready {
        crate::console::warn("Unable to create the AM Daemon console");
    }
    if let Err(error) = std::env::set_current_dir(&base_dir) {
        crate::console::warn(&format!(
            "Unable to set working directory {base_dir}: {error}"
        ));
    }
    match std::panic::catch_unwind(initialize_hooks) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            crate::console::error(&format!("Compatibility initialization failed: {error}"));
            crate::crash::report_startup_failure(&error);
            windows_sys::Win32::System::Threading::ExitProcess(1);
        }
        Err(error) => {
            let detail = if let Some(message) = error.downcast_ref::<&str>() {
                *message
            } else if let Some(message) = error.downcast_ref::<String>() {
                message.as_str()
            } else {
                "unknown Rust panic"
            };
            crate::console::error(&format!("Compatibility initialization panicked: {detail}"));
            crate::crash::report_startup_failure(detail);
            windows_sys::Win32::System::Threading::ExitProcess(1);
        }
    }
}

fn initialize_hooks() -> Result<(), String> {
    crate::console::info("AM Daemon compatibility initialization started");

    let base_dir = executable_base_dir();
    pin_dll("D3DCompiler_43.dll");
    pin_dll("dbghelp.dll");
    crate::command_line::prepare(&base_dir);
    winhttp::amdaemon::initialize(
        &base_dir,
        crate::console::standalone_logger,
        AMDAEMON_MODULE_ORDER,
    )?;
    crate::amvideo_loader::install();
    unsafe { crate::exit_trace::install() };
    crate::console::info("AM Daemon compatibility initialization completed");
    Ok(())
}

// 模块顺序必须满足平台、网络和硬件模拟之间的初始化依赖
const AMDAEMON_MODULE_ORDER: &[&str] = &[
    "platform::dvd::init",
    "iohook::proc_addr::init",
    "platform::reg_hook::init",
    "iohook::init_core",
    "iohook::serial::init",
    "platform::amvideo::init",
    "platform::clock::init",
    "::dns::init",
    "::hwmon::init",
    "platform::misc::init",
    "::netenv::init",
    "::nusec::init",
    "platform::pcbid::init",
    "platform::vfs::init",
    "::epay::init",
    "platform::system::init",
    "::openssl::init",
    "::ewf::init",
    "iohook::init_all",
    "chuniio::init",
    "io4::init",
    "slider::init",
    "vfd::init",
    "led::init",
    "aime::init",
];

fn pin_dll(name: &str) {
    let mut wide = name.encode_utf16().collect::<Vec<_>>();
    wide.push(0);
    let module = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr()) };
    if module.is_null() {
        crate::console::warn(&format!("Failed to load dependency: {name}"));
    } else {
        crate::console::info(&format!("Dependency loaded: {name}"));
    }
}

pub(crate) fn executable_base_dir() -> String {
    let mut path = vec![0u16; 32768];
    let length = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
            std::ptr::null_mut(),
            path.as_mut_ptr(),
            path.len() as u32,
        )
    } as usize;
    if length == 0 || length >= path.len() {
        return ".".to_owned();
    }
    let path = String::from_utf16_lossy(&path[..length]);
    std::path::Path::new(&path)
        .parent()
        .and_then(std::path::Path::to_str)
        .filter(|parent| !parent.is_empty())
        .unwrap_or(".")
        .to_owned()
}

unsafe fn restore_entry() {
    let entry = ENTRY.load(Ordering::Acquire);
    if entry.is_null() {
        return;
    }
    let Some(original) = ORIGINAL.get() else {
        crate::console::error("Unable to restore the original AM Daemon entry point");
        windows_sys::Win32::System::Threading::ExitProcess(1);
    };

    let mut old_protect = 0;
    if VirtualProtect(
        entry.cast(),
        ENTRY_JUMP_LEN,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        crate::console::error("Unable to restore the original AM Daemon entry point");
        windows_sys::Win32::System::Threading::ExitProcess(1);
    }
    // SAFETY: entry 仍是 install 时保存的 EXE 入口，目标内存已由 VirtualProtect 临时设为可写
    ptr::copy_nonoverlapping(original.as_ptr(), entry, ENTRY_JUMP_LEN);
    let mut ignored = 0;
    let _ = VirtualProtect(entry.cast(), ENTRY_JUMP_LEN, old_protect, &mut ignored);
    flush(entry.cast(), ENTRY_JUMP_LEN);
}

unsafe fn flush(address: *const c_void, length: usize) {
    let _ = FlushInstructionCache(GetCurrentProcess(), address, length);
}

unsafe fn entry_point(base: usize) -> Option<usize> {
    if ptr::read_unaligned(base as *const u16) != 0x5A4D {
        return None;
    }
    let nt_offset = ptr::read_unaligned((base + 0x3C) as *const u32) as usize;
    let nt = base.checked_add(nt_offset)?;
    if ptr::read_unaligned(nt as *const u32) != 0x0000_4550 {
        return None;
    }
    let rva = ptr::read_unaligned((nt + 0x28) as *const u32) as usize;
    (rva != 0).then_some(base + rva)
}
