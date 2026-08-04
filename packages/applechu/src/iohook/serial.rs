use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Foundation::HANDLE;

use super::hook_table::{hook_table_apply, null_module, HookSymbol};
use super::proc_addr;
use super::uart::{
    SerialBaudRate, SerialChars, SerialHandflow, SerialLineControl, SerialStatus, SerialTimeouts,
    IOCTL_SERIAL_CLR_DTR, IOCTL_SERIAL_CLR_RTS, IOCTL_SERIAL_GET_BAUD_RATE, IOCTL_SERIAL_GET_CHARS,
    IOCTL_SERIAL_GET_COMMSTATUS, IOCTL_SERIAL_GET_HANDFLOW, IOCTL_SERIAL_GET_LINE_CONTROL,
    IOCTL_SERIAL_GET_MODEMSTATUS, IOCTL_SERIAL_GET_TIMEOUTS, IOCTL_SERIAL_GET_WAIT_MASK,
    IOCTL_SERIAL_PURGE, IOCTL_SERIAL_SET_BAUD_RATE, IOCTL_SERIAL_SET_BREAK_OFF,
    IOCTL_SERIAL_SET_BREAK_ON, IOCTL_SERIAL_SET_CHARS, IOCTL_SERIAL_SET_DTR,
    IOCTL_SERIAL_SET_HANDFLOW, IOCTL_SERIAL_SET_LINE_CONTROL, IOCTL_SERIAL_SET_QUEUE_SIZE,
    IOCTL_SERIAL_SET_RTS, IOCTL_SERIAL_SET_TIMEOUTS, IOCTL_SERIAL_SET_WAIT_MASK,
    IOCTL_SERIAL_SET_XOFF, IOCTL_SERIAL_SET_XON,
};
use crate::util::api::Api;

const ERROR_INVALID_PARAMETER: u32 = 87;

const CLRBREAK: u32 = 9;
const CLRDTR: u32 = 6;
const CLRRTS: u32 = 4;
const SETBREAK: u32 = 8;
const SETDTR: u32 = 5;
const SETRTS: u32 = 3;
const SETXOFF: u32 = 1;
const SETXON: u32 = 2;

const SERIAL_DTR_CONTROL: u32 = 0x0000_0001;
const SERIAL_DTR_HANDSHAKE: u32 = 0x0000_0002;
const SERIAL_ERROR_CHAR: u32 = 0x0000_0004;
const SERIAL_CTS_HANDSHAKE: u32 = 0x0000_0008;
const SERIAL_DSR_HANDSHAKE: u32 = 0x0000_0010;
const SERIAL_DSR_SENSITIVITY: u32 = 0x0000_0040;
const SERIAL_RTS_CONTROL: u32 = 0x0000_0040;
const SERIAL_RTS_HANDSHAKE: u32 = 0x0000_0080;
const SERIAL_NULL_STRIPPING: u32 = 0x0000_0008;
const SERIAL_XOFF_CONTINUE: u32 = 0x8000_0000;
const SERIAL_ERROR_ABORT: u32 = 0x8000_0000;

const SERIAL_ERROR_QUEUEOVERRUN: u32 = 0x0000_0002;
const SERIAL_ERROR_OVERRUN: u32 = 0x0000_0004;
const SERIAL_ERROR_BREAK: u32 = 0x0000_0008;
const SERIAL_ERROR_PARITY: u32 = 0x0000_0001;
const SERIAL_ERROR_FRAMING: u32 = 0x0000_0010;
const SERIAL_TX_WAITING_FOR_CTS: u32 = 0x0000_0001;
const SERIAL_TX_WAITING_FOR_DSR: u32 = 0x0000_0002;
const SERIAL_TX_WAITING_FOR_DCD: u32 = 0x0000_0004;
const SERIAL_TX_WAITING_FOR_XON: u32 = 0x0000_0008;
const SERIAL_TX_WAITING_XOFF_SENT: u32 = 0x0000_0020;

const CE_RXOVER: u32 = 0x0001;
const CE_OVERRUN: u32 = 0x0002;
const CE_RXPARITY: u32 = 0x0004;
const CE_FRAME: u32 = 0x0008;
const CE_BREAK: u32 = 0x0010;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct SerialQueueSize {
    in_size: u32,
    out_size: u32,
}

#[applechu_macros::config_section(stage = PlatformCore, order = 30)]
pub fn init(api: &Api) {
    unsafe {
        let symbols = serial_symbols();
        let patched = hook_table_apply(null_module(), "kernel32.dll", &symbols);
        proc_addr::push("kernel32.dll", &symbols, serial_links_updated);
        api.log_info(&format!(
            "Serial compatibility ready with {patched} patched entries"
        ));
    }
}

unsafe fn serial_symbols() -> [HookSymbol; 13] {
    [
        symbol("ClearCommError", hooked_clear_comm_error as *const ()),
        symbol(
            "EscapeCommFunction",
            hooked_escape_comm_function as *const (),
        ),
        symbol("GetCommMask", hooked_get_comm_mask as *const ()),
        symbol("GetCommState", hooked_get_comm_state as *const ()),
        symbol("GetCommTimeouts", hooked_get_comm_timeouts as *const ()),
        symbol("PurgeComm", hooked_purge_comm as *const ()),
        symbol("SetCommMask", hooked_set_comm_mask as *const ()),
        symbol("SetCommState", hooked_set_comm_state as *const ()),
        symbol("SetCommTimeouts", hooked_set_comm_timeouts as *const ()),
        symbol("SetupComm", hooked_setup_comm as *const ()),
        symbol("ClearCommBreak", hooked_clear_comm_break as *const ()),
        symbol("SetCommBreak", hooked_set_comm_break as *const ()),
        symbol(
            "GetCommModemStatus",
            hooked_get_comm_modem_status as *const (),
        ),
    ]
}

fn symbol(name: &'static str, patch: *const ()) -> HookSymbol {
    HookSymbol {
        name,
        patch,
        original: ptr::null_mut(),
    }
}

fn serial_links_updated() {}

unsafe extern "system" fn hooked_clear_comm_error(
    handle: usize,
    errors: *mut u32,
    status: *mut ComStat,
) -> i32 {
    let mut low_level = SerialStatus::default();
    let hr = invoke_ioctl(
        handle,
        IOCTL_SERIAL_GET_COMMSTATUS,
        ptr::null(),
        0,
        ptr::addr_of_mut!(low_level).cast(),
        std::mem::size_of::<SerialStatus>() as u32,
    );
    if finish(hr, false) == 0 {
        return 0;
    }

    if !errors.is_null() {
        *errors = 0;
        if low_level.errors & SERIAL_ERROR_QUEUEOVERRUN != 0 {
            *errors |= CE_OVERRUN;
        }
        if low_level.errors & SERIAL_ERROR_OVERRUN != 0 {
            *errors |= CE_RXOVER;
        }
        if low_level.errors & SERIAL_ERROR_BREAK != 0 {
            *errors |= CE_BREAK;
        }
        if low_level.errors & SERIAL_ERROR_PARITY != 0 {
            *errors |= CE_RXPARITY;
        }
        if low_level.errors & SERIAL_ERROR_FRAMING != 0 {
            *errors |= CE_FRAME;
        }
    }

    if !status.is_null() {
        *status = ComStat::default();
        if low_level.hold_reasons & SERIAL_TX_WAITING_FOR_CTS != 0 {
            (*status).flags |= 1 << 0;
        }
        if low_level.hold_reasons & SERIAL_TX_WAITING_FOR_DSR != 0 {
            (*status).flags |= 1 << 1;
        }
        if low_level.hold_reasons & SERIAL_TX_WAITING_FOR_DCD != 0 {
            (*status).flags |= 1 << 2;
        }
        if low_level.hold_reasons & SERIAL_TX_WAITING_FOR_XON != 0 {
            (*status).flags |= 1 << 3;
        }
        if low_level.hold_reasons & SERIAL_TX_WAITING_XOFF_SENT != 0 {
            (*status).flags |= 1 << 4;
        }
        if low_level.eof_received != 0 {
            (*status).flags |= 1 << 5;
        }
        if low_level.wait_for_immediate != 0 {
            (*status).flags |= 1 << 6;
        }
        (*status).cb_in_que = low_level.amount_in_in_queue;
        (*status).cb_out_que = low_level.amount_in_out_queue;
    }

    1
}

unsafe extern "system" fn hooked_escape_comm_function(handle: usize, command: u32) -> i32 {
    let ioctl = match command {
        CLRBREAK => IOCTL_SERIAL_SET_BREAK_OFF,
        CLRDTR => IOCTL_SERIAL_CLR_DTR,
        CLRRTS => IOCTL_SERIAL_CLR_RTS,
        SETBREAK => IOCTL_SERIAL_SET_BREAK_ON,
        SETDTR => IOCTL_SERIAL_SET_DTR,
        SETRTS => IOCTL_SERIAL_SET_RTS,
        SETXOFF => IOCTL_SERIAL_SET_XOFF,
        SETXON => IOCTL_SERIAL_SET_XON,
        _ => {
            super::set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    };
    finish(
        invoke_ioctl(handle, ioctl, ptr::null(), 0, ptr::null_mut(), 0),
        false,
    )
}

unsafe extern "system" fn hooked_get_comm_mask(handle: usize, output: *mut u32) -> i32 {
    if output.is_null() {
        super::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut mask = 0u32;
    let hr = invoke_ioctl(
        handle,
        IOCTL_SERIAL_GET_WAIT_MASK,
        ptr::null(),
        0,
        ptr::addr_of_mut!(mask).cast(),
        std::mem::size_of::<u32>() as u32,
    );
    // 该 IOCTL 只验证参数，不把临时 mask 写回调用者
    finish(hr, true)
}

unsafe extern "system" fn hooked_set_comm_mask(handle: usize, mask: u32) -> i32 {
    finish(
        invoke_input(handle, IOCTL_SERIAL_SET_WAIT_MASK, &mask),
        true,
    )
}

unsafe extern "system" fn hooked_get_comm_modem_status(handle: usize, status: *mut u32) -> i32 {
    finish(
        invoke_ioctl(
            handle,
            IOCTL_SERIAL_GET_MODEMSTATUS,
            ptr::null(),
            0,
            status.cast(),
            std::mem::size_of::<u32>() as u32,
        ),
        true,
    )
}

unsafe extern "system" fn hooked_get_comm_state(handle: usize, dcb: *mut Dcb) -> i32 {
    if dcb.is_null() {
        super::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    let mut baud = SerialBaudRate { baud_rate: 0 };
    let mut handflow = SerialHandflow::default();
    let mut line = SerialLineControl {
        stop_bits: 0,
        parity: 0,
        word_length: 0,
    };
    let mut chars = SerialChars {
        eof_char: 0,
        error_char: 0,
        break_char: 0,
        event_char: 0,
        xon_char: 0,
        xoff_char: 0,
    };

    for hr in [
        invoke_output(handle, IOCTL_SERIAL_GET_BAUD_RATE, &mut baud),
        invoke_output(handle, IOCTL_SERIAL_GET_HANDFLOW, &mut handflow),
        invoke_output(handle, IOCTL_SERIAL_GET_LINE_CONTROL, &mut line),
        invoke_output(handle, IOCTL_SERIAL_GET_CHARS, &mut chars),
    ] {
        if finish(hr, false) == 0 {
            return 0;
        }
    }

    *dcb = Dcb::default();
    (*dcb).dcblength = std::mem::size_of::<Dcb>() as u32;
    (*dcb).baud_rate = baud.baud_rate;
    (*dcb).flags = 1;
    if handflow.control_handshake & SERIAL_CTS_HANDSHAKE != 0 {
        (*dcb).flags |= 1 << 2;
    }
    if handflow.control_handshake & SERIAL_DSR_HANDSHAKE != 0 {
        (*dcb).flags |= 1 << 3;
    }
    if handflow.control_handshake & SERIAL_DTR_CONTROL != 0 {
        (*dcb).flags |= 1 << 4;
    }
    if handflow.control_handshake & SERIAL_DTR_HANDSHAKE != 0 {
        (*dcb).flags |= 2 << 4;
    }
    if handflow.control_handshake & SERIAL_DSR_SENSITIVITY != 0 {
        (*dcb).flags |= 1 << 6;
    }
    if handflow.control_handshake & SERIAL_XOFF_CONTINUE != 0 {
        (*dcb).flags |= 1 << 7;
    }
    // 读取时从 ControlHandShake 判断 RTS，写入时保存在 FlowReplace
    if handflow.control_handshake & SERIAL_RTS_CONTROL != 0 {
        (*dcb).flags |= 1 << 12;
    }
    if handflow.control_handshake & SERIAL_RTS_HANDSHAKE != 0 {
        (*dcb).flags |= 2 << 12;
    }
    if handflow.control_handshake & SERIAL_ERROR_ABORT != 0 {
        (*dcb).flags |= 1 << 14;
    }
    if handflow.control_handshake & SERIAL_ERROR_CHAR != 0 {
        (*dcb).flags |= 1 << 10;
    }
    if handflow.control_handshake & SERIAL_NULL_STRIPPING != 0 {
        (*dcb).flags |= 1 << 11;
    }
    (*dcb).xon_lim = handflow.xon_limit as u16;
    (*dcb).xoff_lim = handflow.xoff_limit as u16;
    (*dcb).byte_size = line.word_length;
    (*dcb).parity = line.parity;
    (*dcb).stop_bits = line.stop_bits;
    (*dcb).xon_char = chars.xon_char as i8;
    (*dcb).xoff_char = chars.xoff_char as i8;
    (*dcb).error_char = chars.error_char as i8;
    (*dcb).eof_char = chars.eof_char as i8;
    (*dcb).evt_char = chars.event_char as i8;
    super::set_last_error(0);
    1
}

unsafe extern "system" fn hooked_set_comm_state(handle: usize, dcb: *const Dcb) -> i32 {
    if dcb.is_null() || (*dcb).dcblength != std::mem::size_of::<Dcb>() as u32 {
        super::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    let baud = SerialBaudRate {
        baud_rate: (*dcb).baud_rate,
    };
    let mut handflow = SerialHandflow::default();
    if (*dcb).flags & (1 << 2) != 0 {
        handflow.control_handshake |= SERIAL_CTS_HANDSHAKE;
    }
    if (*dcb).flags & (1 << 3) != 0 {
        handflow.control_handshake |= SERIAL_DSR_HANDSHAKE;
    }
    match ((*dcb).flags >> 4) & 0x3 {
        0 => {}
        1 => handflow.control_handshake |= SERIAL_DTR_CONTROL,
        2 => handflow.control_handshake |= SERIAL_DTR_HANDSHAKE,
        _ => {
            super::set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    }
    if (*dcb).flags & (1 << 6) != 0 {
        handflow.control_handshake |= SERIAL_DSR_SENSITIVITY;
    }
    if (*dcb).flags & (1 << 7) != 0 {
        handflow.control_handshake |= SERIAL_XOFF_CONTINUE;
    }
    match ((*dcb).flags >> 12) & 0x3 {
        0 => {}
        1 => handflow.flow_replace |= SERIAL_RTS_CONTROL,
        2 => handflow.flow_replace |= SERIAL_RTS_HANDSHAKE,
        _ => {
            super::set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    }
    handflow.xon_limit = (*dcb).xon_lim as i32;
    handflow.xoff_limit = (*dcb).xoff_lim as i32;

    let line = SerialLineControl {
        stop_bits: (*dcb).stop_bits,
        parity: (*dcb).parity,
        word_length: (*dcb).byte_size,
    };
    let chars = SerialChars {
        eof_char: (*dcb).eof_char as u8,
        error_char: (*dcb).error_char as u8,
        break_char: 0,
        event_char: (*dcb).evt_char as u8,
        xon_char: (*dcb).xon_char as u8,
        xoff_char: (*dcb).xoff_char as u8,
    };

    for hr in [
        invoke_input(handle, IOCTL_SERIAL_SET_BAUD_RATE, &baud),
        invoke_input(handle, IOCTL_SERIAL_SET_HANDFLOW, &handflow),
        invoke_input(handle, IOCTL_SERIAL_SET_LINE_CONTROL, &line),
        invoke_input(handle, IOCTL_SERIAL_SET_CHARS, &chars),
    ] {
        if finish(hr, false) == 0 {
            return 0;
        }
    }
    super::set_last_error(0);
    1
}

unsafe extern "system" fn hooked_get_comm_timeouts(
    handle: usize,
    timeouts: *mut CommTimeouts,
) -> i32 {
    if timeouts.is_null() {
        super::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let mut serial = SerialTimeouts::default();
    let hr = invoke_output(handle, IOCTL_SERIAL_GET_TIMEOUTS, &mut serial);
    if finish(hr, false) == 0 {
        return 0;
    }
    *timeouts = CommTimeouts::from(serial);
    super::set_last_error(0);
    1
}

unsafe extern "system" fn hooked_set_comm_timeouts(
    handle: usize,
    timeouts: *const CommTimeouts,
) -> i32 {
    if timeouts.is_null() {
        super::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    finish(
        invoke_input(
            handle,
            IOCTL_SERIAL_SET_TIMEOUTS,
            &SerialTimeouts::from(*timeouts),
        ),
        true,
    )
}

unsafe extern "system" fn hooked_purge_comm(handle: usize, flags: u32) -> i32 {
    finish(invoke_input(handle, IOCTL_SERIAL_PURGE, &flags), true)
}

unsafe extern "system" fn hooked_setup_comm(handle: usize, in_queue: u32, out_queue: u32) -> i32 {
    let queue = SerialQueueSize {
        in_size: in_queue,
        out_size: out_queue,
    };
    finish(
        invoke_input(handle, IOCTL_SERIAL_SET_QUEUE_SIZE, &queue),
        true,
    )
}

unsafe extern "system" fn hooked_clear_comm_break(handle: usize) -> i32 {
    finish(
        invoke_ioctl(
            handle,
            IOCTL_SERIAL_SET_BREAK_OFF,
            ptr::null(),
            0,
            ptr::null_mut(),
            0,
        ),
        true,
    )
}

unsafe extern "system" fn hooked_set_comm_break(handle: usize) -> i32 {
    finish(
        invoke_ioctl(
            handle,
            IOCTL_SERIAL_SET_BREAK_ON,
            ptr::null(),
            0,
            ptr::null_mut(),
            0,
        ),
        true,
    )
}

unsafe fn invoke_input<T>(handle: usize, code: u32, input: &T) -> i32 {
    invoke_ioctl(
        handle,
        code,
        (input as *const T).cast(),
        std::mem::size_of::<T>() as u32,
        ptr::null_mut(),
        0,
    )
}

unsafe fn invoke_output<T>(handle: usize, code: u32, output: &mut T) -> i32 {
    invoke_ioctl(
        handle,
        code,
        ptr::null(),
        0,
        (output as *mut T).cast(),
        std::mem::size_of::<T>() as u32,
    )
}

unsafe fn invoke_ioctl(
    handle: usize,
    code: u32,
    input: *const c_void,
    input_size: u32,
    output: *mut c_void,
    output_size: u32,
) -> i32 {
    let mut returned = 0;
    let mut irp = super::make_fd_irp(
        super::IrpOp::Ioctl,
        handle as HANDLE,
        ptr::null_mut(),
        &mut returned,
    );
    irp.ioctl = code;
    irp.ioctl_in = input.cast_mut();
    irp.ioctl_in_nbytes = input_size;
    irp.ioctl_out = output;
    irp.ioctl_out_nbytes = output_size;
    super::invoke_next(&mut irp)
}

unsafe fn finish(hr: i32, clear_last_error: bool) -> i32 {
    if super::failed(hr) {
        super::propagate_hresult(hr);
        0
    } else {
        if clear_last_error {
            super::set_last_error(0);
        }
        1
    }
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

const _: () = assert!(std::mem::size_of::<Dcb>() == 28);
const _: () = assert!(std::mem::size_of::<CommTimeouts>() == 20);
const _: () = assert!(std::mem::size_of::<ComStat>() == 12);
