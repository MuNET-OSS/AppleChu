pub mod hook_table;
pub mod iobuf;
pub mod proc_addr;
pub mod serial;
pub mod setupapi;
pub mod uart;

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;

use windows_sys::Win32::Foundation::{BOOL, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Threading::{
    EnterCriticalSection, InitializeCriticalSection, LeaveCriticalSection, SetEvent,
    CRITICAL_SECTION,
};
use windows_sys::Win32::System::IO::OVERLAPPED;

use crate::config::Config;
use crate::util::api::Api;

use self::hook_table::{hook_table_apply, null_module, HookSymbol};

const MAX_HANDLERS: usize = 32;
const KERNEL32_DLL: &str = "kernel32.dll";
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_GEN_FAILURE: u32 = 31;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_OUTOFMEMORY: u32 = 14;
const ERROR_OPERATION_ABORTED: u32 = 995;
const ERROR_INVALID_ADDRESS: u32 = 487;
const ERROR_INTERNAL_ERROR: u32 = 1359;
pub const ERROR_IO_PENDING: u32 = 997;
const STATUS_SUCCESS: usize = 0;
pub const S_OK: i32 = 0;
pub const E_FAIL: i32 = 0x8000_4005_u32 as i32;
pub const E_PENDING: i32 = 0x8000_000A_u32 as i32;
const E_ABORT: i32 = 0x8000_4004_u32 as i32;
const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
const E_HANDLE: i32 = 0x8007_0006_u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
const E_NOTIMPL: i32 = 0x8000_4001_u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000E_u32 as i32;
const E_POINTER: i32 = 0x8000_4003_u32 as i32;
const E_UNEXPECTED: i32 = 0x8000_FFFF_u32 as i32;

type CreateFileAFn =
    unsafe extern "system" fn(*const u8, u32, u32, *const c_void, u32, u32, HANDLE) -> HANDLE;
type CreateFileWFn =
    unsafe extern "system" fn(*const u16, u32, u32, *const c_void, u32, u32, HANDLE) -> HANDLE;
type ReadFileFn =
    unsafe extern "system" fn(HANDLE, *mut c_void, u32, *mut u32, *mut c_void) -> BOOL;
type WriteFileFn =
    unsafe extern "system" fn(HANDLE, *const c_void, u32, *mut u32, *mut c_void) -> BOOL;
type DeviceIoControlFn = unsafe extern "system" fn(
    HANDLE,
    u32,
    *mut c_void,
    u32,
    *mut c_void,
    u32,
    *mut u32,
    *mut c_void,
) -> BOOL;
type CloseHandleFn = unsafe extern "system" fn(HANDLE) -> BOOL;
type SetFilePointerFn = unsafe extern "system" fn(HANDLE, i32, *mut i32, u32) -> u32;
type SetFilePointerExFn = unsafe extern "system" fn(HANDLE, i64, *mut i64, u32) -> BOOL;
type FlushFileBuffersFn = unsafe extern "system" fn(HANDLE) -> BOOL;

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(module_name: *const u16) -> HANDLE;
    fn GetProcAddress(module: HANDLE, proc_name: *const u8) -> *const ();
    fn SetLastError(error: u32);
    fn GetLastError() -> u32;
}

pub const fn hresult_from_win32(error: u32) -> i32 {
    if error == 0 {
        S_OK
    } else {
        (0x8007_0000 | (error & 0xFFFF)) as i32
    }
}

const fn failed(hr: i32) -> bool {
    hr < 0
}

unsafe fn propagate_hresult(hr: i32) {
    let error = if (hr as u32 & 0xFFFF_0000) == 0x8007_0000 {
        hr as u32 & 0xFFFF
    } else {
        match hr {
            E_ABORT => ERROR_OPERATION_ABORTED,
            E_ACCESSDENIED => ERROR_ACCESS_DENIED,
            E_FAIL => ERROR_GEN_FAILURE,
            E_HANDLE => ERROR_INVALID_HANDLE,
            E_INVALIDARG => ERROR_INVALID_PARAMETER,
            E_NOINTERFACE => ERROR_INVALID_FUNCTION,
            E_NOTIMPL => ERROR_NOT_SUPPORTED,
            E_OUTOFMEMORY => ERROR_OUTOFMEMORY,
            E_PENDING => ERROR_IO_PENDING,
            E_POINTER => ERROR_INVALID_ADDRESS,
            E_UNEXPECTED => ERROR_INTERNAL_ERROR,
            _ => ERROR_INTERNAL_ERROR,
        }
    };
    SetLastError(error);
}

pub type IrpHandler = unsafe fn(&mut Irp) -> i32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IrpOp {
    Open,
    Close,
    Read,
    Write,
    Ioctl,
    Fsync,
    Seek,
}

pub struct Irp {
    pub op: IrpOp,
    pub fd: HANDLE,
    pub ovl: *mut c_void,
    pub read_buf: *mut u8,
    pub write_buf: *const u8,
    pub nbytes: u32,
    pub out_nbytes: *mut u32,
    pub ioctl: u32,
    pub ioctl_in: *mut c_void,
    pub ioctl_in_nbytes: u32,
    pub ioctl_out: *mut c_void,
    pub ioctl_out_nbytes: u32,
    pub open_filename_a: *const u8,
    pub open_filename_w: *const u16,
    pub open_access: u32,
    pub open_share: u32,
    pub open_security: *const c_void,
    pub open_creation: u32,
    pub open_flags: u32,
    pub open_template: HANDLE,
    pub seek_distance: i64,
    pub seek_distance_high: *mut i32,
    pub seek_method: u32,
    pub seek_result: *mut i64,
    next_handler: usize,
}

struct HandlerState {
    handlers: [Option<IrpHandler>; MAX_HANDLERS],
    count: usize,
}

struct CriticalSectionLock {
    init: Once,
    cs: UnsafeCell<CRITICAL_SECTION>,
}

unsafe impl Sync for CriticalSectionLock {}

impl CriticalSectionLock {
    const fn new() -> Self {
        Self {
            init: Once::new(),
            cs: UnsafeCell::new(unsafe { std::mem::zeroed() }),
        }
    }

    unsafe fn lock(&self) -> CriticalSectionGuard<'_> {
        self.init
            .call_once(|| unsafe { InitializeCriticalSection(self.cs.get()) });
        EnterCriticalSection(self.cs.get());
        CriticalSectionGuard { lock: self }
    }
}

struct CriticalSectionGuard<'a> {
    lock: &'a CriticalSectionLock,
}

impl Drop for CriticalSectionGuard<'_> {
    fn drop(&mut self) {
        unsafe { LeaveCriticalSection(self.lock.cs.get()) };
    }
}

static HANDLER_LOCK: CriticalSectionLock = CriticalSectionLock::new();
static INSTALL_ONCE: Once = Once::new();
static INSTALLED_PATCHES: AtomicUsize = AtomicUsize::new(0);
static mut HANDLERS: HandlerState = HandlerState {
    handlers: [None; MAX_HANDLERS],
    count: 0,
};

static mut ORIGINAL_CREATE_FILE_A: *const () = ptr::null();
static mut ORIGINAL_CREATE_FILE_W: *const () = ptr::null();
static mut ORIGINAL_READ_FILE: *const () = ptr::null();
static mut ORIGINAL_WRITE_FILE: *const () = ptr::null();
static mut ORIGINAL_DEVICE_IO_CONTROL: *const () = ptr::null();
static mut ORIGINAL_CLOSE_HANDLE: *const () = ptr::null();
static mut ORIGINAL_SET_FILE_POINTER: *const () = ptr::null();
static mut ORIGINAL_SET_FILE_POINTER_EX: *const () = ptr::null();
static mut ORIGINAL_FLUSH_FILE_BUFFERS: *const () = ptr::null();

#[applechu_macros::config_section(stage = IoHook, order = 10)]
pub fn init_all(api: &Api, config: &Config) {
    setupapi::init(api);
    uart::init_all(api, config);
}

#[applechu_macros::config_section(stage = PlatformCore, order = 25)]
pub fn init_core(api: &Api, _config: &Config) {
    unsafe {
        let patched = install();
        api.log_info(&format!(
            "Device I/O compatibility ready with {patched} patched entries"
        ));
    }
}

pub unsafe fn push_handler(handler: IrpHandler) -> bool {
    let _guard = HANDLER_LOCK.lock();
    if HANDLERS.count >= MAX_HANDLERS {
        return false;
    }
    let idx = HANDLERS.count;
    HANDLERS.handlers[idx] = Some(handler);
    HANDLERS.count += 1;
    true
}

pub unsafe fn open_nul_fd() -> Option<HANDLE> {
    // 平台设备可能早于显式 IoHook 阶段调用这里，因此需要惰性初始化
    install();
    let create_file: CreateFileWFn = resolve_original(ORIGINAL_CREATE_FILE_W, b"CreateFileW\0");
    let handle = create_file(
        [b'N' as u16, b'U' as u16, b'L' as u16, 0].as_ptr(),
        0xC000_0000,
        3,
        ptr::null(),
        3,
        0x4000_0000,
        ptr::null_mut(),
    );
    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

pub unsafe fn set_last_error(error: u32) {
    SetLastError(error);
}

pub unsafe fn invoke_next(irp: &mut Irp) -> i32 {
    let handler = {
        let _guard = HANDLER_LOCK.lock();
        let idx = irp.next_handler;
        if idx < HANDLERS.count {
            irp.next_handler += 1;
            HANDLERS.handlers[idx]
        } else {
            None
        }
    };

    let hr = match handler {
        Some(handler) => handler(irp),
        None => fallthrough(irp),
    };
    if failed(hr) {
        irp.next_handler = usize::MAX;
    }
    hr
}

pub unsafe fn install() -> usize {
    INSTALL_ONCE.call_once(|| unsafe {
        let symbols = [
            HookSymbol {
                name: "CreateFileA",
                patch: hooked_create_file_a as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_CREATE_FILE_A),
            },
            HookSymbol {
                name: "CreateFileW",
                patch: hooked_create_file_w as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_CREATE_FILE_W),
            },
            HookSymbol {
                name: "ReadFile",
                patch: hooked_read_file as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_READ_FILE),
            },
            HookSymbol {
                name: "WriteFile",
                patch: hooked_write_file as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_WRITE_FILE),
            },
            HookSymbol {
                name: "DeviceIoControl",
                patch: hooked_device_io_control as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_DEVICE_IO_CONTROL),
            },
            HookSymbol {
                name: "CloseHandle",
                patch: hooked_close_handle as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_CLOSE_HANDLE),
            },
            HookSymbol {
                name: "SetFilePointer",
                patch: hooked_set_file_pointer as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_SET_FILE_POINTER),
            },
            HookSymbol {
                name: "SetFilePointerEx",
                patch: hooked_set_file_pointer_ex as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_SET_FILE_POINTER_EX),
            },
            HookSymbol {
                name: "FlushFileBuffers",
                patch: hooked_flush_file_buffers as *const (),
                original: ptr::addr_of_mut!(ORIGINAL_FLUSH_FILE_BUFFERS),
            },
        ];
        let patched = hook_table_apply(null_module(), KERNEL32_DLL, &symbols);
        proc_addr::push(KERNEL32_DLL, &symbols, sync_originals);
        INSTALLED_PATCHES.store(patched, Ordering::Release);
    });
    INSTALLED_PATCHES.load(Ordering::Acquire)
}

fn sync_originals() {}

unsafe fn fallthrough(irp: &mut Irp) -> i32 {
    match irp.op {
        IrpOp::Open if !irp.open_filename_a.is_null() => {
            let original: CreateFileAFn =
                resolve_original(ORIGINAL_CREATE_FILE_A, b"CreateFileA\0");
            let handle = original(
                irp.open_filename_a,
                irp.open_access,
                irp.open_share,
                irp.open_security,
                irp.open_creation,
                irp.open_flags,
                irp.open_template,
            );
            irp.fd = handle;
            if handle == INVALID_HANDLE_VALUE {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Open => {
            let original: CreateFileWFn =
                resolve_original(ORIGINAL_CREATE_FILE_W, b"CreateFileW\0");
            let handle = original(
                irp.open_filename_w,
                irp.open_access,
                irp.open_share,
                irp.open_security,
                irp.open_creation,
                irp.open_flags,
                irp.open_template,
            );
            irp.fd = handle;
            if handle == INVALID_HANDLE_VALUE {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Close => {
            let original: CloseHandleFn = resolve_original(ORIGINAL_CLOSE_HANDLE, b"CloseHandle\0");
            if original(irp.fd) == 0 {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Read => {
            let original: ReadFileFn = resolve_original(ORIGINAL_READ_FILE, b"ReadFile\0");
            let mut transferred = 0;
            let ok = original(
                irp.fd,
                irp.read_buf.cast(),
                irp.nbytes,
                &mut transferred,
                irp.ovl,
            );
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = transferred;
            }
            if ok == 0 {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Write => {
            let original: WriteFileFn = resolve_original(ORIGINAL_WRITE_FILE, b"WriteFile\0");
            let mut transferred = 0;
            let ok = original(
                irp.fd,
                irp.write_buf.cast(),
                irp.nbytes,
                &mut transferred,
                irp.ovl,
            );
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = transferred;
            }
            if ok == 0 {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Ioctl => {
            let original: DeviceIoControlFn =
                resolve_original(ORIGINAL_DEVICE_IO_CONTROL, b"DeviceIoControl\0");
            let mut transferred = 0;
            let ok = original(
                irp.fd,
                irp.ioctl,
                irp.ioctl_in,
                irp.ioctl_in_nbytes,
                irp.ioctl_out,
                irp.ioctl_out_nbytes,
                &mut transferred,
                irp.ovl,
            );
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = transferred;
            }
            if ok == 0 {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Fsync => {
            let original: FlushFileBuffersFn =
                resolve_original(ORIGINAL_FLUSH_FILE_BUFFERS, b"FlushFileBuffers\0");
            if original(irp.fd) == 0 {
                hresult_from_win32(GetLastError())
            } else {
                S_OK
            }
        }
        IrpOp::Seek => {
            let original: SetFilePointerExFn =
                resolve_original(ORIGINAL_SET_FILE_POINTER_EX, b"SetFilePointerEx\0");
            let mut new_pos: i64 = 0;
            let ok = original(irp.fd, irp.seek_distance, &mut new_pos, irp.seek_method);
            if ok == 0 {
                return hresult_from_win32(GetLastError());
            }
            if !irp.seek_result.is_null() {
                *irp.seek_result = new_pos;
            }
            S_OK
        }
    }
}

unsafe fn resolve_original<T>(saved: *const (), name: &[u8]) -> T {
    if !saved.is_null() {
        return std::mem::transmute_copy(&saved);
    }
    static K32: &[u16] = &[
        b'k' as u16,
        b'e' as u16,
        b'r' as u16,
        b'n' as u16,
        b'e' as u16,
        b'l' as u16,
        b'3' as u16,
        b'2' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    let k32 = GetModuleHandleW(K32.as_ptr());
    let proc = GetProcAddress(k32, name.as_ptr());
    std::mem::transmute_copy(&proc)
}

unsafe extern "system" fn hooked_create_file_a(
    filename: *const u8,
    access: u32,
    share: u32,
    security: *const c_void,
    creation: u32,
    flags: u32,
    template: HANDLE,
) -> HANDLE {
    let mut irp = make_open_irp(
        filename,
        ptr::null(),
        access,
        share,
        security,
        creation,
        flags,
        template,
    );
    let hr = invoke_next(&mut irp);
    if failed(hr) {
        propagate_hresult(hr);
        INVALID_HANDLE_VALUE
    } else {
        set_last_error(0);
        irp.fd
    }
}

unsafe extern "system" fn hooked_create_file_w(
    filename: *const u16,
    access: u32,
    share: u32,
    security: *const c_void,
    creation: u32,
    flags: u32,
    template: HANDLE,
) -> HANDLE {
    let mut irp = make_open_irp(
        ptr::null(),
        filename,
        access,
        share,
        security,
        creation,
        flags,
        template,
    );
    let hr = invoke_next(&mut irp);
    if failed(hr) {
        propagate_hresult(hr);
        INVALID_HANDLE_VALUE
    } else {
        set_last_error(0);
        irp.fd
    }
}

unsafe fn make_open_irp(
    filename_a: *const u8,
    filename_w: *const u16,
    access: u32,
    share: u32,
    security: *const c_void,
    creation: u32,
    flags: u32,
    template: HANDLE,
) -> Irp {
    Irp {
        op: IrpOp::Open,
        fd: INVALID_HANDLE_VALUE,
        ovl: ptr::null_mut(),
        read_buf: ptr::null_mut(),
        write_buf: ptr::null(),
        nbytes: 0,
        out_nbytes: ptr::null_mut(),
        ioctl: 0,
        ioctl_in: ptr::null_mut(),
        ioctl_in_nbytes: 0,
        ioctl_out: ptr::null_mut(),
        ioctl_out_nbytes: 0,
        open_filename_a: filename_a,
        open_filename_w: filename_w,
        open_access: access,
        open_share: share,
        open_security: security,
        open_creation: creation,
        open_flags: flags,
        open_template: template,
        seek_distance: 0,
        seek_distance_high: ptr::null_mut(),
        seek_method: 0,
        seek_result: ptr::null_mut(),
        next_handler: 0,
    }
}

unsafe extern "system" fn hooked_read_file(
    fd: HANDLE,
    buf: *mut c_void,
    nbytes: u32,
    out_nbytes: *mut u32,
    ovl: *mut c_void,
) -> BOOL {
    if fd.is_null() || fd == INVALID_HANDLE_VALUE || buf.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if ovl.is_null() && out_nbytes.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if ovl.is_null() {
        *out_nbytes = 0;
    }
    let mut transferred = 0;
    let reported = &mut transferred;
    let mut irp = make_fd_irp(IrpOp::Read, fd, ovl, reported);
    irp.read_buf = buf.cast();
    irp.nbytes = nbytes;
    let result = invoke_next(&mut irp);
    if result == E_PENDING {
        if !out_nbytes.is_null() {
            *out_nbytes = 0;
        }
        return complete_overlapped(ptr::null_mut(), ovl, 0);
    }
    if failed(result) {
        propagate_hresult(result);
        return 0;
    }
    complete_overlapped(out_nbytes, ovl, transferred)
}

unsafe extern "system" fn hooked_write_file(
    fd: HANDLE,
    buf: *const c_void,
    nbytes: u32,
    out_nbytes: *mut u32,
    ovl: *mut c_void,
) -> BOOL {
    if fd.is_null() || fd == INVALID_HANDLE_VALUE || buf.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if ovl.is_null() && out_nbytes.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if ovl.is_null() {
        *out_nbytes = 0;
    }
    let mut transferred = 0;
    let reported = &mut transferred;
    let mut irp = make_fd_irp(IrpOp::Write, fd, ovl, reported);
    irp.write_buf = buf.cast();
    irp.nbytes = nbytes;
    let hr = invoke_next(&mut irp);
    if failed(hr) {
        propagate_hresult(hr);
        return 0;
    }
    complete_overlapped(out_nbytes, ovl, transferred)
}

unsafe extern "system" fn hooked_device_io_control(
    fd: HANDLE,
    ioctl: u32,
    in_buf: *mut c_void,
    in_nbytes: u32,
    out_buf: *mut c_void,
    out_nbytes: u32,
    returned: *mut u32,
    ovl: *mut c_void,
) -> BOOL {
    if fd.is_null() || fd == INVALID_HANDLE_VALUE {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    if ovl.is_null() {
        if returned.is_null() {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        *returned = 0;
    }

    let mut transferred = 0;
    let mut irp = make_fd_irp(IrpOp::Ioctl, fd, ovl, &mut transferred);
    irp.ioctl = ioctl;
    irp.ioctl_in = in_buf;
    irp.ioctl_in_nbytes = in_nbytes;
    irp.ioctl_out = out_buf;
    irp.ioctl_out_nbytes = out_nbytes;
    let result = invoke_next(&mut irp);
    if failed(result) {
        // ERROR_MORE_DATA 等失败仍可能返回有效字节数
        if !returned.is_null() {
            *returned = transferred;
        }
        propagate_hresult(result);
        return 0;
    }
    complete_overlapped(returned, ovl, transferred)
}

unsafe fn complete_overlapped(returned: *mut u32, ovl: *mut c_void, transferred: u32) -> BOOL {
    if !ovl.is_null() {
        let ovl = ovl.cast::<OVERLAPPED>();
        (*ovl).Internal = STATUS_SUCCESS;
        (*ovl).InternalHigh = transferred as usize;
        if !(*ovl).hEvent.is_null() {
            SetEvent((*ovl).hEvent);
        }
    }

    if returned.is_null() {
        set_last_error(ERROR_IO_PENDING);
        0
    } else {
        *returned = transferred;
        set_last_error(0);
        1
    }
}

unsafe extern "system" fn hooked_close_handle(fd: HANDLE) -> BOOL {
    if fd.is_null() || fd == INVALID_HANDLE_VALUE {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut irp = make_fd_irp(IrpOp::Close, fd, ptr::null_mut(), ptr::null_mut());
    let hr = invoke_next(&mut irp);
    if failed(hr) {
        propagate_hresult(hr);
        0
    } else {
        1
    }
}

unsafe extern "system" fn hooked_flush_file_buffers(fd: HANDLE) -> BOOL {
    if fd.is_null() || fd == INVALID_HANDLE_VALUE {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut irp = make_fd_irp(IrpOp::Fsync, fd, ptr::null_mut(), ptr::null_mut());
    let hr = invoke_next(&mut irp);
    if failed(hr) {
        propagate_hresult(hr);
        0
    } else {
        set_last_error(0);
        1
    }
}

unsafe extern "system" fn hooked_set_file_pointer(
    fd: HANDLE,
    distance: i32,
    distance_high: *mut i32,
    method: u32,
) -> u32 {
    const INVALID_SET_FILE_POINTER: u32 = 0xFFFF_FFFF;
    if fd.is_null() || fd == INVALID_HANDLE_VALUE {
        set_last_error(ERROR_INVALID_PARAMETER);
        return INVALID_SET_FILE_POINTER;
    }
    let mut irp = make_fd_irp(IrpOp::Seek, fd, ptr::null_mut(), ptr::null_mut());
    irp.seek_distance = if distance_high.is_null() {
        distance as i64
    } else {
        ((*distance_high as i64) << 32) | (distance as u32 as i64)
    };
    irp.seek_distance_high = distance_high;
    irp.seek_method = method;
    let mut result_pos: i64 = 0;
    irp.seek_result = &mut result_pos;
    let hr = invoke_next(&mut irp);
    if failed(hr) {
        propagate_hresult(hr);
        return INVALID_SET_FILE_POINTER;
    }
    if !distance_high.is_null() {
        *distance_high = (result_pos >> 32) as i32;
    }
    set_last_error(0);
    result_pos as u32
}

unsafe extern "system" fn hooked_set_file_pointer_ex(
    fd: HANDLE,
    distance: i64,
    new_pointer: *mut i64,
    method: u32,
) -> BOOL {
    if fd.is_null() || fd == INVALID_HANDLE_VALUE {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut irp = make_fd_irp(IrpOp::Seek, fd, ptr::null_mut(), ptr::null_mut());
    irp.seek_distance = distance;
    irp.seek_method = method;
    let mut result_pos: i64 = 0;
    irp.seek_result = &mut result_pos;
    let result = invoke_next(&mut irp);
    if failed(result) {
        propagate_hresult(result);
        return 0;
    }
    if !new_pointer.is_null() {
        *new_pointer = result_pos;
    }
    set_last_error(0);
    1
}

fn make_fd_irp(op: IrpOp, fd: HANDLE, ovl: *mut c_void, out_nbytes: *mut u32) -> Irp {
    Irp {
        op,
        fd,
        ovl,
        read_buf: ptr::null_mut(),
        write_buf: ptr::null(),
        nbytes: 0,
        out_nbytes,
        ioctl: 0,
        ioctl_in: ptr::null_mut(),
        ioctl_in_nbytes: 0,
        ioctl_out: ptr::null_mut(),
        ioctl_out_nbytes: 0,
        open_filename_a: ptr::null(),
        open_filename_w: ptr::null(),
        open_access: 0,
        open_share: 0,
        open_security: ptr::null(),
        open_creation: 0,
        open_flags: 0,
        open_template: ptr::null_mut(),
        seek_distance: 0,
        seek_distance_high: ptr::null_mut(),
        seek_method: 0,
        seek_result: ptr::null_mut(),
        next_handler: 0,
    }
}
