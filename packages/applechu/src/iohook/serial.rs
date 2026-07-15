use std::sync::atomic::{AtomicUsize, Ordering};

use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

use super::uart::{self, SerialLineControl, SerialTimeouts};

type CommBoolFn = unsafe extern "system" fn(usize) -> i32;
type ClearCommErrorFn = unsafe extern "system" fn(usize, *mut u32, *mut ComStat) -> i32;
type EscapeCommFunctionFn = unsafe extern "system" fn(usize, u32) -> i32;
type GetCommStateFn = unsafe extern "system" fn(usize, *mut Dcb) -> i32;
type SetCommStateFn = unsafe extern "system" fn(usize, *const Dcb) -> i32;
type GetCommTimeoutsFn = unsafe extern "system" fn(usize, *mut CommTimeouts) -> i32;
type SetCommTimeoutsFn = unsafe extern "system" fn(usize, *const CommTimeouts) -> i32;
type PurgeCommFn = unsafe extern "system" fn(usize, u32) -> i32;
type SetupCommFn = unsafe extern "system" fn(usize, u32, u32) -> i32;

static ORIG_CLEAR_COMM_ERROR: AtomicUsize = AtomicUsize::new(0);
static ORIG_ESCAPE_COMM_FUNCTION: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_COMM_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIG_SET_COMM_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_COMM_TIMEOUTS: AtomicUsize = AtomicUsize::new(0);
static ORIG_SET_COMM_TIMEOUTS: AtomicUsize = AtomicUsize::new(0);
static ORIG_PURGE_COMM: AtomicUsize = AtomicUsize::new(0);
static ORIG_SETUP_COMM: AtomicUsize = AtomicUsize::new(0);
static ORIG_CLEAR_COMM_BREAK: AtomicUsize = AtomicUsize::new(0);
static ORIG_SET_COMM_BREAK: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Dcb {
    pub dcblength: u32,
    pub baud_rate: u32,
    pub flags: u32,
    pub w_reserved: u16,
    pub xon_lim: u16,
    pub xoff_lim: u16,
    pub byte_size: u8,
    pub parity: u8,
    pub stop_bits: u8,
    pub xon_char: i8,
    pub xoff_char: i8,
    pub error_char: i8,
    pub eof_char: i8,
    pub evt_char: i8,
    pub w_reserved1: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CommTimeouts {
    pub read_interval_timeout: u32,
    pub read_total_timeout_multiplier: u32,
    pub read_total_timeout_constant: u32,
    pub write_total_timeout_multiplier: u32,
    pub write_total_timeout_constant: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ComStat {
    pub flags: u32,
    pub cb_in_que: u32,
    pub cb_out_que: u32,
}

pub fn init(api: &Api) {
    let base = api.game_base();
    unsafe {
        install_hook(
            api,
            base,
            "ClearCommError",
            hooked_clear_comm_error as *const (),
            &ORIG_CLEAR_COMM_ERROR,
        );
        install_hook(
            api,
            base,
            "EscapeCommFunction",
            hooked_escape_comm_function as *const (),
            &ORIG_ESCAPE_COMM_FUNCTION,
        );
        install_hook(
            api,
            base,
            "GetCommState",
            hooked_get_comm_state as *const (),
            &ORIG_GET_COMM_STATE,
        );
        install_hook(
            api,
            base,
            "SetCommState",
            hooked_set_comm_state as *const (),
            &ORIG_SET_COMM_STATE,
        );
        install_hook(
            api,
            base,
            "GetCommTimeouts",
            hooked_get_comm_timeouts as *const (),
            &ORIG_GET_COMM_TIMEOUTS,
        );
        install_hook(
            api,
            base,
            "SetCommTimeouts",
            hooked_set_comm_timeouts as *const (),
            &ORIG_SET_COMM_TIMEOUTS,
        );
        install_hook(
            api,
            base,
            "PurgeComm",
            hooked_purge_comm as *const (),
            &ORIG_PURGE_COMM,
        );
        install_hook(
            api,
            base,
            "SetupComm",
            hooked_setup_comm as *const (),
            &ORIG_SETUP_COMM,
        );
        install_hook(
            api,
            base,
            "ClearCommBreak",
            hooked_clear_comm_break as *const (),
            &ORIG_CLEAR_COMM_BREAK,
        );
        install_hook(
            api,
            base,
            "SetCommBreak",
            hooked_set_comm_break as *const (),
            &ORIG_SET_COMM_BREAK,
        );
    }
}

unsafe fn install_hook(api: &Api, base: usize, name: &str, detour: *const (), slot: &AtomicUsize) {
    if let Some(original) = hook_iat(base, "kernel32.dll", name, detour) {
        slot.store(original as usize, Ordering::SeqCst);
        api.log_info(&format!("iohook: serial {name} hooked"));
    }
}

unsafe extern "system" fn hooked_clear_comm_error(
    handle: usize,
    errors: *mut u32,
    stat: *mut ComStat,
) -> i32 {
    if uart::is_uart_handle(handle) {
        if !errors.is_null() {
            *errors = 0;
        }
        if !stat.is_null() {
            *stat = ComStat::default();
        }
        return 1;
    }
    call_clear_comm_error(handle, errors, stat)
}

unsafe extern "system" fn hooked_escape_comm_function(handle: usize, function: u32) -> i32 {
    if uart::is_uart_handle(handle) {
        return uart::set_escape(handle, function) as i32;
    }
    let original_addr = ORIG_ESCAPE_COMM_FUNCTION.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: EscapeCommFunctionFn = std::mem::transmute(original_addr);
    original(handle, function)
}

unsafe extern "system" fn hooked_get_comm_state(handle: usize, dcb: *mut Dcb) -> i32 {
    if uart::is_uart_handle(handle) {
        if dcb.is_null() {
            return 0;
        }
        let Some(state) = uart::snapshot(handle) else {
            return 0;
        };
        (*dcb).dcblength = std::mem::size_of::<Dcb>() as u32;
        (*dcb).baud_rate = state.baud_rate;
        (*dcb).byte_size = state.line_control.word_length;
        (*dcb).parity = state.line_control.parity;
        (*dcb).stop_bits = state.line_control.stop_bits;
        return 1;
    }
    let original_addr = ORIG_GET_COMM_STATE.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: GetCommStateFn = std::mem::transmute(original_addr);
    original(handle, dcb)
}

unsafe extern "system" fn hooked_set_comm_state(handle: usize, dcb: *const Dcb) -> i32 {
    if uart::is_uart_handle(handle) {
        if dcb.is_null() {
            return 0;
        }
        let dcb = *dcb;
        uart::set_baud_rate(handle, dcb.baud_rate);
        uart::set_line_control(
            handle,
            SerialLineControl {
                stop_bits: dcb.stop_bits,
                parity: dcb.parity,
                word_length: dcb.byte_size,
            },
        ) as i32
    } else {
        let original_addr = ORIG_SET_COMM_STATE.load(Ordering::SeqCst);
        if original_addr == 0 {
            return 0;
        }
        let original: SetCommStateFn = std::mem::transmute(original_addr);
        original(handle, dcb)
    }
}

unsafe extern "system" fn hooked_get_comm_timeouts(
    handle: usize,
    timeouts: *mut CommTimeouts,
) -> i32 {
    if uart::is_uart_handle(handle) {
        if timeouts.is_null() {
            return 0;
        }
        let Some(state) = uart::snapshot(handle) else {
            return 0;
        };
        *timeouts = CommTimeouts::from(state.timeouts);
        return 1;
    }
    let original_addr = ORIG_GET_COMM_TIMEOUTS.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: GetCommTimeoutsFn = std::mem::transmute(original_addr);
    original(handle, timeouts)
}

unsafe extern "system" fn hooked_set_comm_timeouts(
    handle: usize,
    timeouts: *const CommTimeouts,
) -> i32 {
    if uart::is_uart_handle(handle) {
        if timeouts.is_null() {
            return 0;
        }
        return uart::set_timeouts(handle, SerialTimeouts::from(*timeouts)) as i32;
    }
    let original_addr = ORIG_SET_COMM_TIMEOUTS.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: SetCommTimeoutsFn = std::mem::transmute(original_addr);
    original(handle, timeouts)
}

unsafe extern "system" fn hooked_purge_comm(handle: usize, flags: u32) -> i32 {
    if uart::is_uart_handle(handle) {
        let _ = flags;
        return uart::purge(handle) as i32;
    }
    let original_addr = ORIG_PURGE_COMM.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: PurgeCommFn = std::mem::transmute(original_addr);
    original(handle, flags)
}

unsafe extern "system" fn hooked_setup_comm(handle: usize, in_queue: u32, out_queue: u32) -> i32 {
    if uart::is_uart_handle(handle) {
        let _ = (in_queue, out_queue);
        return 1;
    }
    let original_addr = ORIG_SETUP_COMM.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: SetupCommFn = std::mem::transmute(original_addr);
    original(handle, in_queue, out_queue)
}

unsafe extern "system" fn hooked_clear_comm_break(handle: usize) -> i32 {
    if uart::is_uart_handle(handle) {
        return 1;
    }
    call_comm_bool(handle, &ORIG_CLEAR_COMM_BREAK)
}

unsafe extern "system" fn hooked_set_comm_break(handle: usize) -> i32 {
    if uart::is_uart_handle(handle) {
        return 1;
    }
    call_comm_bool(handle, &ORIG_SET_COMM_BREAK)
}

unsafe fn call_clear_comm_error(handle: usize, errors: *mut u32, stat: *mut ComStat) -> i32 {
    let original_addr = ORIG_CLEAR_COMM_ERROR.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: ClearCommErrorFn = std::mem::transmute(original_addr);
    original(handle, errors, stat)
}

unsafe fn call_comm_bool(handle: usize, slot: &AtomicUsize) -> i32 {
    let original_addr = slot.load(Ordering::SeqCst);
    if original_addr == 0 {
        return 0;
    }
    let original: CommBoolFn = std::mem::transmute(original_addr);
    original(handle)
}

impl From<SerialTimeouts> for CommTimeouts {
    fn from(value: SerialTimeouts) -> Self {
        Self {
            read_interval_timeout: value.read_interval_timeout,
            read_total_timeout_multiplier: value.read_total_timeout_multiplier,
            read_total_timeout_constant: value.read_total_timeout_constant,
            write_total_timeout_multiplier: value.write_total_timeout_multiplier,
            write_total_timeout_constant: value.write_total_timeout_constant,
        }
    }
}

impl From<CommTimeouts> for SerialTimeouts {
    fn from(value: CommTimeouts) -> Self {
        Self {
            read_interval_timeout: value.read_interval_timeout,
            read_total_timeout_multiplier: value.read_total_timeout_multiplier,
            read_total_timeout_constant: value.read_total_timeout_constant,
            write_total_timeout_multiplier: value.write_total_timeout_multiplier,
            write_total_timeout_constant: value.write_total_timeout_constant,
        }
    }
}
