use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use once_cell::sync::Lazy;
use windows_sys::Win32::Devices::Communication::{
    GetCommState, SetCommState, SetCommTimeouts, COMMTIMEOUTS, DCB, NOPARITY, ONESTOPBIT,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, WriteFile, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::Sleep;

const FRAMING: u8 = 0xE0;
const ESCAPE: u8 = 0xD0;
const BOARD_COUNT: usize = 3;
const BOARD_DATA_LENS: [usize; BOARD_COUNT] = [53 * 3, 63 * 3, 31 * 3];
const PIPE_NAME: &str = "\\\\.\\pipe\\chuni_led";
const PIPE_ACCESS_OUTBOUND: u32 = 0x0000_0002;
const GENERIC_WRITE: u32 = 0x4000_0000;

crate::config_section! {
    pub(crate) struct LedOutputConfig => LED_OUTPUT_CONFIG_SECTION {
        section: "Led",
        order: 90,
        default_on: true,
        always_enabled: false,
        hidden: false,
        group: "io",
        comment: "灯光输出",
        fields: {
            pub cab_pipe: bool = true,
            key: "cabLedOutputPipe",
            advanced: true,
            comment: "通过命名管道输出机台灯光";
            pub cab_serial: bool = false,
            key: "cabLedOutputSerial",
            advanced: true,
            comment: "通过串口输出机台灯光";
            pub controller_pipe: bool = true,
            key: "controllerLedOutputPipe",
            advanced: true,
            comment: "通过命名管道输出控制器灯光";
            pub controller_serial: bool = false,
            key: "controllerLedOutputSerial",
            advanced: true,
            comment: "通过串口输出控制器灯光";
            pub controller_openithm: bool = false,
            key: "controllerLedOutputOpeNITHM",
            advanced: true,
            comment: "使用 OpeNITHM 控制器灯光格式";
            pub serial_port: String = String::from("COM5"),
            advanced: true,
            comment: "灯光输出串口";
            pub serial_baud: u32 = 921_600,
            advanced: true,
            comment: "灯光输出串口波特率";
        }
    }
}

struct LedOutputState {
    config: LedOutputConfig,
    pipe_updates: [Option<Vec<u8>>; BOARD_COUNT],
    serial_handle: HANDLE,
}

unsafe impl Send for LedOutputState {}

static STATE: Lazy<Mutex<Option<LedOutputState>>> = Lazy::new(|| Mutex::new(None));
static PIPE_STARTED: AtomicBool = AtomicBool::new(false);

pub fn init(config: LedOutputConfig) {
    let any_pipe = config.cab_pipe || config.controller_pipe;
    let any_serial = config.cab_serial || config.controller_serial;
    let serial_handle = if any_serial {
        open_serial(&config.serial_port, config.serial_baud)
    } else {
        INVALID_HANDLE_VALUE
    };

    if let Ok(mut state) = STATE.lock() {
        *state = Some(LedOutputState {
            config,
            pipe_updates: [None, None, None],
            serial_handle,
        });
    }

    if any_pipe && !PIPE_STARTED.swap(true, Ordering::SeqCst) {
        let _ = thread::Builder::new()
            .name("chuni-led-pipe".to_owned())
            .spawn(pipe_thread);
    }
}

pub fn update(board: u8, rgb: &[u8]) {
    let board = board as usize;
    if board >= BOARD_COUNT {
        return;
    }
    let data_len = BOARD_DATA_LENS[board].min(rgb.len());
    let escaped = packet(board as u8, &rgb[..data_len]);

    let mut openithm = None;
    if board == 2 {
        let mut frame = Vec::with_capacity(100);
        frame.extend_from_slice(&[0xAA, 0xAA]);
        frame.extend_from_slice(&rgb[..rgb.len().min(96)]);
        frame.resize(98, 0);
        frame.extend_from_slice(&[0xDD, 0xDD]);
        openithm = Some(frame);
    }

    if let Ok(mut guard) = STATE.lock() {
        let Some(state) = guard.as_mut() else {
            return;
        };

        if (board < 2 && state.config.cab_pipe) || (board == 2 && state.config.controller_pipe) {
            state.pipe_updates[board] = Some(escaped.clone());
        }

        if board < 2 && state.config.cab_serial {
            write_serial(state.serial_handle, &escaped);
        } else if board == 2 && state.config.controller_serial {
            if state.config.controller_openithm {
                if let Some(frame) = openithm.as_ref() {
                    write_serial(state.serial_handle, frame);
                }
            } else {
                write_serial(state.serial_handle, &escaped);
            }
        }
    }
}

fn packet(board: u8, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + rgb.len() * 2);
    out.push(FRAMING);
    out.push(board);
    for byte in rgb.iter().copied() {
        if byte == FRAMING || byte == ESCAPE {
            out.push(ESCAPE);
            out.push(byte.wrapping_sub(1));
        } else {
            out.push(byte);
        }
    }
    out
}

fn pipe_thread() {
    loop {
        let pipe = create_pipe();
        if pipe == INVALID_HANDLE_VALUE {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) != 0 };
        if connected {
            loop {
                let mut wrote = true;
                if let Ok(mut guard) = STATE.lock() {
                    if let Some(state) = guard.as_mut() {
                        for slot in &mut state.pipe_updates {
                            if let Some(data) = slot.take() {
                                if !write_handle(pipe, &data) {
                                    wrote = false;
                                    break;
                                }
                            }
                        }
                    }
                }
                if !wrote {
                    break;
                }
                unsafe { Sleep(1) };
            }
        }

        unsafe {
            DisconnectNamedPipe(pipe);
            CloseHandle(pipe);
        }
    }
}

fn create_pipe() -> HANDLE {
    let name = wide(PIPE_NAME);
    unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_OUTBOUND,
            PIPE_TYPE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            2 + 66 * 3 * 2,
            0,
            0,
            ptr::null_mut(),
        )
    }
}

fn open_serial(port: &str, baud: u32) -> HANDLE {
    let path = if port.starts_with("\\\\.\\") {
        port.to_owned()
    } else {
        format!("\\\\.\\{port}")
    };
    let wide = wide(&path);
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE,
            0,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        );
        if handle != INVALID_HANDLE_VALUE {
            configure_serial(handle, baud);
        }
        handle
    }
}

fn configure_serial(handle: HANDLE, baud: u32) {
    unsafe {
        let mut dcb = std::mem::zeroed::<DCB>();
        dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
        GetCommState(handle, &mut dcb);
        dcb.BaudRate = baud;
        dcb.ByteSize = 8;
        dcb.StopBits = ONESTOPBIT;
        dcb.Parity = NOPARITY;
        SetCommState(handle, &dcb);

        let timeouts = COMMTIMEOUTS {
            ReadIntervalTimeout: 50,
            ReadTotalTimeoutMultiplier: 10,
            ReadTotalTimeoutConstant: 50,
            WriteTotalTimeoutMultiplier: 10,
            WriteTotalTimeoutConstant: 50,
        };
        SetCommTimeouts(handle, &timeouts);
    }
}

fn write_serial(handle: HANDLE, data: &[u8]) {
    if handle == INVALID_HANDLE_VALUE {
        return;
    }
    write_handle(handle, data);
}

fn write_handle(handle: HANDLE, data: &[u8]) -> bool {
    let mut written = 0u32;
    unsafe {
        WriteFile(
            handle,
            data.as_ptr(),
            data.len() as u32,
            &mut written,
            ptr::null_mut(),
        ) != 0
            && written == data.len() as u32
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
