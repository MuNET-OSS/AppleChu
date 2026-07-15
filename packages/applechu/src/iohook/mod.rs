pub mod hook_table;
pub mod iobuf;
pub mod proc_addr;
pub mod serial;
pub mod setupapi;
pub mod uart;

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Once;

use windows_sys::Win32::Foundation::{BOOL, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Threading::{
    CRITICAL_SECTION, EnterCriticalSection, InitializeCriticalSection, LeaveCriticalSection,
};

use crate::config::Config;
use crate::util::api::Api;

use self::hook_table::{HookSymbol, hook_table_apply, null_module};

const MAX_HANDLERS: usize = 32;
const KERNEL32_DLL: &str = "kernel32.dll";

fn log_diag(msg: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(msg);
    }
}

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
    unsafe {
        let patched = install();
        if patched != 0 {
            api.log_info(&format!("iohook: kernel32 hooks installed ({})", patched));
            api.log_info(&format!(
                "iohook: ORIG_CreateFileA={:#x}, ORIG_CreateFileW={:#x}",
                ORIGINAL_CREATE_FILE_A as usize, ORIGINAL_CREATE_FILE_W as usize
            ));
        }
    }
    setupapi::init(api);
    uart::init_all(api, config);
    serial::init(api);
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
    let create_file: CreateFileAFn = resolve_original(ORIGINAL_CREATE_FILE_A, b"CreateFileA\0");
    let handle = create_file(
        b"NUL\0".as_ptr(),
        0xC000_0000,
        3,
        ptr::null(),
        3,
        0,
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

    match handler {
        Some(handler) => handler(irp),
        None => fallthrough(irp),
    }
}

pub unsafe fn install() -> usize {
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
    hook_table_apply(null_module(), KERNEL32_DLL, &symbols)
}

unsafe fn fallthrough(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open && !irp.open_filename_a.is_null() {
        let name = std::ffi::CStr::from_ptr(irp.open_filename_a.cast()).to_string_lossy();
        log_diag(&format!("iohook fallthrough OPEN(A): {}", name));
    }
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
            (handle != INVALID_HANDLE_VALUE) as i32
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
            (handle != INVALID_HANDLE_VALUE) as i32
        }
        IrpOp::Close => {
            let original: CloseHandleFn = resolve_original(ORIGINAL_CLOSE_HANDLE, b"CloseHandle\0");
            original(irp.fd)
        }
        IrpOp::Read => {
            let original: ReadFileFn = resolve_original(ORIGINAL_READ_FILE, b"ReadFile\0");
            original(
                irp.fd,
                irp.read_buf.cast(),
                irp.nbytes,
                irp.out_nbytes,
                irp.ovl,
            )
        }
        IrpOp::Write => {
            let original: WriteFileFn = resolve_original(ORIGINAL_WRITE_FILE, b"WriteFile\0");
            original(
                irp.fd,
                irp.write_buf.cast(),
                irp.nbytes,
                irp.out_nbytes,
                irp.ovl,
            )
        }
        IrpOp::Ioctl => {
            let original: DeviceIoControlFn =
                resolve_original(ORIGINAL_DEVICE_IO_CONTROL, b"DeviceIoControl\0");
            original(
                irp.fd,
                irp.ioctl,
                irp.ioctl_in,
                irp.ioctl_in_nbytes,
                irp.ioctl_out,
                irp.ioctl_out_nbytes,
                irp.out_nbytes,
                irp.ovl,
            )
        }
        IrpOp::Fsync => {
            let original: FlushFileBuffersFn =
                resolve_original(ORIGINAL_FLUSH_FILE_BUFFERS, b"FlushFileBuffers\0");
            original(irp.fd)
        }
        IrpOp::Seek => {
            let original: SetFilePointerExFn =
                resolve_original(ORIGINAL_SET_FILE_POINTER_EX, b"SetFilePointerEx\0");
            let mut new_pos: i64 = 0;
            let ok = original(irp.fd, irp.seek_distance, &mut new_pos, irp.seek_method);
            if ok != 0 && !irp.seek_result.is_null() {
                *irp.seek_result = new_pos;
            }
            ok
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
    if invoke_next(&mut irp) == 0 {
        INVALID_HANDLE_VALUE
    } else {
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
    if invoke_next(&mut irp) == 0 {
        INVALID_HANDLE_VALUE
    } else {
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
    let mut irp = make_fd_irp(IrpOp::Read, fd, ovl, out_nbytes);
    irp.read_buf = buf.cast();
    irp.nbytes = nbytes;
    invoke_next(&mut irp)
}

unsafe extern "system" fn hooked_write_file(
    fd: HANDLE,
    buf: *const c_void,
    nbytes: u32,
    out_nbytes: *mut u32,
    ovl: *mut c_void,
) -> BOOL {
    let mut irp = make_fd_irp(IrpOp::Write, fd, ovl, out_nbytes);
    irp.write_buf = buf.cast();
    irp.nbytes = nbytes;
    invoke_next(&mut irp)
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
    let mut irp = make_fd_irp(IrpOp::Ioctl, fd, ovl, returned);
    irp.ioctl = ioctl;
    irp.ioctl_in = in_buf;
    irp.ioctl_in_nbytes = in_nbytes;
    irp.ioctl_out = out_buf;
    irp.ioctl_out_nbytes = out_nbytes;
    invoke_next(&mut irp)
}

unsafe extern "system" fn hooked_close_handle(fd: HANDLE) -> BOOL {
    let mut irp = make_fd_irp(IrpOp::Close, fd, ptr::null_mut(), ptr::null_mut());
    invoke_next(&mut irp)
}

unsafe extern "system" fn hooked_flush_file_buffers(fd: HANDLE) -> BOOL {
    let mut irp = make_fd_irp(IrpOp::Fsync, fd, ptr::null_mut(), ptr::null_mut());
    invoke_next(&mut irp)
}

unsafe extern "system" fn hooked_set_file_pointer(
    fd: HANDLE,
    distance: i32,
    distance_high: *mut i32,
    method: u32,
) -> u32 {
    const INVALID_SET_FILE_POINTER: u32 = 0xFFFF_FFFF;
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
    if invoke_next(&mut irp) == 0 {
        return INVALID_SET_FILE_POINTER;
    }
    if !distance_high.is_null() {
        *distance_high = (result_pos >> 32) as i32;
    }
    result_pos as u32
}

unsafe extern "system" fn hooked_set_file_pointer_ex(
    fd: HANDLE,
    distance: i64,
    new_pointer: *mut i64,
    method: u32,
) -> BOOL {
    let mut irp = make_fd_irp(IrpOp::Seek, fd, ptr::null_mut(), ptr::null_mut());
    irp.seek_distance = distance;
    irp.seek_method = method;
    let mut result_pos: i64 = 0;
    irp.seek_result = &mut result_pos;
    let result = invoke_next(&mut irp);
    if result != 0 && !new_pointer.is_null() {
        *new_pointer = result_pos;
    }
    result
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
