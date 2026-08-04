use std::collections::HashMap;
use std::ffi::{c_char, CStr};
use std::ptr;
use std::sync::Mutex;

use once_cell::sync::Lazy;

use applechu::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use applechu::iohook::proc_addr;
use applechu::iohook::{self, Irp, IrpOp};
use applechu::util::api::Api;
use applechu::util::win32::{handle_from_value, handle_value};

const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const TRUNCATE_EXISTING: u32 = 5;
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_INVALID_FUNCTION: u32 = 1;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_NOTIMPL: i32 = 0x8000_4001_u32 as i32;
const S_FALSE: i32 = 1;

const DEFAULT_PATHS: [&str; 6] = [
    "alib.conf",
    "cacert.pem",
    "first_ar.conf",
    "last_pras.log",
    "last_shime.log",
    "play_history.csv",
];

#[derive(Default)]
struct OpenFile {
    path: String,
    position: usize,
}

#[derive(Default)]
struct State {
    files: HashMap<String, Vec<u8>>,
    handles: HashMap<usize, OpenFile>,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));
static mut NEXT_STAT64I32: *const () = ptr::null();

type Stat64I32Fn = unsafe extern "C" fn(*const c_char, *mut Stat64I32) -> i32;

// MSVC 2012 在 x64 下的 _stat64i32 布局。游戏只读取 st_mode 和 st_size，
// 但仍完整初始化结构，确保返回的是有效的普通文件结果
#[repr(C)]
struct Stat64I32 {
    bytes: [u8; 32],
}

#[applechu_macros::config_section(stage = Platform, order = 120)]
pub fn init(api: &Api) {
    unsafe {
        iohook::push_handler(handle_irp);
        install_stat_hook();
    }
    api.log_info("EWF write protection enabled for billing record files");
}

unsafe fn install_stat_hook() {
    let symbols = [
        HookSymbol {
            name: "__imp__stat64i32",
            patch: hooked_stat64i32 as *const (),
            original: ptr::addr_of_mut!(NEXT_STAT64I32),
        },
        HookSymbol {
            name: "_stat64i32",
            patch: hooked_stat64i32 as *const (),
            original: ptr::addr_of_mut!(NEXT_STAT64I32),
        },
    ];
    hook_table_apply(null_module(), "msvcr110.dll", &symbols);
    hook_table_apply(null_module(), "msvcr110d.dll", &symbols);
    proc_addr::push("msvcr110.dll", &symbols, sync_stat_original);
    proc_addr::push("msvcr110d.dll", &symbols, sync_stat_original);
}

fn sync_stat_original() {}

unsafe fn handle_irp(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        let Some(path) = irp_path(irp) else {
            return iohook::invoke_next(irp);
        };
        if !needs_virtualization(&path) {
            return iohook::invoke_next(irp);
        }
        return open(irp, path);
    }

    let handle = handle_value(irp.fd);
    let known = STATE
        .lock()
        .is_ok_and(|state| state.handles.contains_key(&handle));
    if !known {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Read => read(irp, handle),
        IrpOp::Write => write(irp, handle),
        IrpOp::Close => {
            if let Ok(mut state) = STATE.lock() {
                state.handles.remove(&handle);
            }
            iohook::invoke_next(irp)
        }
        IrpOp::Seek | IrpOp::Fsync | IrpOp::Ioctl | IrpOp::Open => {
            iohook::hresult_from_win32(ERROR_INVALID_FUNCTION)
        }
    }
}

unsafe fn open(irp: &mut Irp, path: String) -> i32 {
    if !irp.ovl.is_null() {
        return E_NOTIMPL;
    }
    let canonical = path.to_ascii_lowercase();
    let Ok(mut state) = STATE.lock() else {
        return iohook::E_FAIL;
    };
    let exists = state.files.contains_key(&canonical);
    match irp.open_creation {
        CREATE_NEW if exists => return iohook::hresult_from_win32(ERROR_FILE_EXISTS),
        CREATE_NEW | OPEN_ALWAYS if !exists => {
            state.files.insert(canonical.clone(), Vec::new());
        }
        CREATE_ALWAYS => {
            state.files.insert(canonical.clone(), Vec::new());
        }
        OPEN_EXISTING if !exists => return iohook::hresult_from_win32(ERROR_FILE_NOT_FOUND),
        TRUNCATE_EXISTING if !exists => {
            return iohook::hresult_from_win32(ERROR_FILE_NOT_FOUND);
        }
        TRUNCATE_EXISTING => {
            if let Some(file) = state.files.get_mut(&canonical) {
                file.clear();
            }
        }
        OPEN_EXISTING | OPEN_ALWAYS => {}
        _ => return E_INVALIDARG,
    }

    let Some(fd) = iohook::open_nul_fd() else {
        return iohook::E_FAIL;
    };
    let handle = handle_value(fd);
    state.handles.insert(
        handle,
        OpenFile {
            path: canonical,
            position: 0,
        },
    );
    irp.fd = handle_from_value(handle);
    iohook::S_OK
}

unsafe fn read(irp: &mut Irp, handle: usize) -> i32 {
    if irp.read_buf.is_null() {
        return iohook::hresult_from_win32(87);
    }
    let Ok(mut state) = STATE.lock() else {
        return iohook::E_FAIL;
    };
    let Some(open) = state.handles.remove(&handle) else {
        return iohook::E_FAIL;
    };
    let bytes = state
        .files
        .get(&open.path)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if open.position > bytes.len() {
        state.handles.insert(handle, open);
        return iohook::hresult_from_win32(ERROR_HANDLE_EOF);
    }
    let count = (bytes.len() - open.position).min(irp.nbytes as usize);
    if count != 0 {
        ptr::copy_nonoverlapping(bytes.as_ptr().add(open.position), irp.read_buf, count);
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = count as u32;
    }
    state.handles.insert(
        handle,
        OpenFile {
            position: open.position + count,
            ..open
        },
    );
    if count == 0 {
        S_FALSE
    } else {
        iohook::S_OK
    }
}

unsafe fn write(irp: &mut Irp, handle: usize) -> i32 {
    if irp.write_buf.is_null() {
        return iohook::hresult_from_win32(87);
    }
    let Ok(mut state) = STATE.lock() else {
        return iohook::E_FAIL;
    };
    let Some(open) = state.handles.remove(&handle) else {
        return iohook::E_FAIL;
    };
    let count = irp.nbytes as usize;
    let end = open.position.saturating_add(count);
    let file = state.files.entry(open.path.clone()).or_default();
    if file.len() < end {
        file.resize(end, 0);
    }
    ptr::copy_nonoverlapping(irp.write_buf, file.as_mut_ptr().add(open.position), count);
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = irp.nbytes;
    }
    state.handles.insert(
        handle,
        OpenFile {
            position: end,
            ..open
        },
    );
    iohook::S_OK
}

unsafe extern "C" fn hooked_stat64i32(path: *const c_char, buffer: *mut Stat64I32) -> i32 {
    if path.is_null() || buffer.is_null() {
        return -1;
    }
    let path = CStr::from_ptr(path).to_string_lossy().into_owned();
    let transformed = applechu::platform::vfs::resolve_path(&path)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.clone());
    if !needs_virtualization(&transformed) {
        return call_next_stat(&path, buffer);
    }

    let canonical = transformed.to_ascii_lowercase();
    let Ok(state) = STATE.lock() else {
        return -1;
    };
    let Some(file) = state.files.get(&canonical) else {
        return -1;
    };
    let bytes = &mut (*buffer).bytes;
    bytes.fill(0);
    // st_mode 偏移 4 使用 _S_IFREG，st_size 位于偏移 16
    bytes[4..6].copy_from_slice(&0x8000u16.to_le_bytes());
    bytes[16..24].copy_from_slice(&(file.len() as i64).to_le_bytes());
    0
}

unsafe fn call_next_stat(path: &str, buffer: *mut Stat64I32) -> i32 {
    if !NEXT_STAT64I32.is_null() {
        let next: Stat64I32Fn = std::mem::transmute(NEXT_STAT64I32);
        let path = applechu::platform::winapi::to_cstring_lossy(path);
        return next(path.as_ptr(), buffer);
    }
    -1
}

unsafe fn irp_path(irp: &Irp) -> Option<String> {
    if !irp.open_filename_w.is_null() {
        return applechu::platform::winapi::wide_to_string(irp.open_filename_w);
    }
    if irp.open_filename_a.is_null() {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(irp.open_filename_a.cast())
            .to_string_lossy()
            .into_owned(),
    )
}

fn needs_virtualization(path: &str) -> bool {
    let normalized = path.replace('/', "\\").to_ascii_lowercase();
    DEFAULT_PATHS
        .iter()
        .any(|suffix| normalized.ends_with(suffix))
}
