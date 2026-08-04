use std::ffi::c_void;
use std::mem::{size_of, zeroed};
#[cfg(target_arch = "x86")]
use std::ptr::null;
use std::ptr::null_mut;

#[cfg(target_arch = "x86")]
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::MAX_PATH;
#[cfg(target_arch = "x86")]
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddrModeFlat, RtlCaptureStackBackTrace, StackWalk, SymCleanup, SymFromAddr,
    SymFunctionTableAccess, SymGetModuleBase, SymInitialize, STACKFRAME, SYMBOL_INFO,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetModuleBaseNameA, GetModuleInformation, MODULEINFO,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
#[cfg(target_arch = "x86")]
use windows_sys::Win32::System::Threading::GetCurrentThread;

const IMAGE_FILE_MACHINE_I386: u32 = 0x014c;

/// 捕获当前线程的返回地址，并转换为便于静态分析的模块名与偏移
#[cfg(target_arch = "x86")]
pub(in crate::proxy::loader) unsafe fn append_current_stack_trace(
    out: &mut String,
    frames_to_skip: u32,
) {
    use std::fmt::Write as _;

    let mut frames = [null_mut(); 48];
    let count = RtlCaptureStackBackTrace(
        frames_to_skip.saturating_add(1),
        frames.len() as u32,
        frames.as_mut_ptr(),
        null_mut(),
    ) as usize;

    let _ = writeln!(out, "exit_stack:");
    for (index, &frame) in frames[..count].iter().enumerate() {
        let address = frame as usize;
        let location = module_offset(address)
            .map(|(module, offset)| format!("{module}+0x{offset:X}"))
            .unwrap_or_else(|| "<unknown>".to_string());
        let _ = writeln!(out, "  #{index:02} 0x{address:08X} {location}");
    }
}

#[cfg(target_arch = "x86")]
pub(super) type NativeContext = windows_sys::Win32::System::Diagnostics::Debug::CONTEXT;

#[cfg(target_arch = "x86")]
pub(super) fn append_registers(out: &mut String, ctx: &NativeContext) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "\nregisters:");
    let _ = writeln!(
        out,
        "  EAX=0x{:08X} EBX=0x{:08X} ECX=0x{:08X} EDX=0x{:08X}",
        ctx.Eax, ctx.Ebx, ctx.Ecx, ctx.Edx
    );
    let _ = writeln!(
        out,
        "  ESI=0x{:08X} EDI=0x{:08X} EBP=0x{:08X} ESP=0x{:08X}",
        ctx.Esi, ctx.Edi, ctx.Ebp, ctx.Esp
    );
    let _ = writeln!(out, "  EIP=0x{:08X} EFLAGS=0x{:08X}", ctx.Eip, ctx.EFlags);
}

#[cfg(target_arch = "x86")]
pub(super) unsafe fn append_stack_trace(out: &mut String, ctx: &mut NativeContext) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "\nstack_trace:");
    let process = GetCurrentProcess();
    let thread = GetCurrentThread();
    SymInitialize(process, null(), 1);

    let mut frame: STACKFRAME = zeroed();
    frame.AddrPC.Offset = ctx.Eip;
    frame.AddrPC.Mode = AddrModeFlat;
    frame.AddrFrame.Offset = ctx.Ebp;
    frame.AddrFrame.Mode = AddrModeFlat;
    frame.AddrStack.Offset = ctx.Esp;
    frame.AddrStack.Mode = AddrModeFlat;

    for index in 0..64 {
        let ok = StackWalk(
            IMAGE_FILE_MACHINE_I386,
            process,
            thread,
            &mut frame,
            ctx as *mut _ as *mut c_void,
            None,
            Some(SymFunctionTableAccess),
            Some(SymGetModuleBase),
            None,
        );
        if ok == 0 || frame.AddrPC.Offset == 0 {
            break;
        }
        let addr = frame.AddrPC.Offset;
        let symbol = symbol_from_addr(process, addr as u64).unwrap_or_else(|| {
            module_offset(addr as usize)
                .map(|(module, offset)| format!("{module}+0x{offset:X}"))
                .unwrap_or_else(|| "<unknown>".to_string())
        });
        let _ = writeln!(out, "  #{index:02} 0x{:08X} {symbol}", addr as u32);
    }

    SymCleanup(process);
}

#[cfg(target_arch = "x86")]
unsafe fn symbol_from_addr(process: HANDLE, addr: u64) -> Option<String> {
    let mut storage = [0u8; size_of::<SYMBOL_INFO>() + 512];
    let symbol = storage.as_mut_ptr() as *mut SYMBOL_INFO;
    (*symbol).SizeOfStruct = size_of::<SYMBOL_INFO>() as u32;
    (*symbol).MaxNameLen = 511;
    let mut displacement = 0u64;
    if SymFromAddr(process, addr, &mut displacement, symbol) == 0 {
        return None;
    }
    let name_ptr = (*symbol).Name.as_ptr() as *const u8;
    let len = (0..511).position(|i| *name_ptr.add(i) == 0).unwrap_or(511);
    let name = String::from_utf8_lossy(std::slice::from_raw_parts(name_ptr, len)).into_owned();
    Some(format!("{name}+0x{displacement:X}"))
}

pub(super) unsafe fn module_offset(addr: usize) -> Option<(String, usize)> {
    let module = module_from_address(addr)?;
    let mut name = [0u8; MAX_PATH as usize];
    let len = GetModuleBaseNameA(
        GetCurrentProcess(),
        module,
        name.as_mut_ptr(),
        name.len() as u32,
    );
    let module_name = if len == 0 {
        "<module>".to_string()
    } else {
        String::from_utf8_lossy(&name[..len as usize]).into_owned()
    };
    Some((module_name, addr.saturating_sub(module as usize)))
}

unsafe fn module_from_address(addr: usize) -> Option<*mut c_void> {
    let mut module = null_mut();
    let flags = 0x00000004u32 | 0x00000002u32;
    let ok = windows_sys::Win32::System::LibraryLoader::GetModuleHandleExA(
        flags,
        addr as *const u8,
        &mut module,
    );
    if ok == 0 || module.is_null() {
        return None;
    }
    let mut info: MODULEINFO = zeroed();
    if GetModuleInformation(
        GetCurrentProcess(),
        module,
        &mut info,
        size_of::<MODULEINFO>() as u32,
    ) == 0
    {
        return None;
    }
    Some(module)
}
