use std::ffi::{CStr, CString, OsString, c_char, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};

use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const INVALID_HANDLE_VALUE: usize = usize::MAX;
pub const REG_BINARY: u32 = 3;
pub const REG_DWORD: u32 = 4;
pub const REG_SZ: u32 = 1;

pub type CreateFileAFn =
    unsafe extern "system" fn(*const c_char, u32, u32, *const c_void, u32, u32, usize) -> usize;
pub type CreateFileWFn =
    unsafe extern "system" fn(*const u16, u32, u32, *const c_void, u32, u32, usize) -> usize;
pub type CloseHandleFn = unsafe extern "system" fn(usize) -> i32;
pub type DeviceIoControlFn = unsafe extern "system" fn(
    usize,
    u32,
    *mut c_void,
    u32,
    *mut c_void,
    u32,
    *mut u32,
    *mut c_void,
) -> i32;
pub type ExitWindowsExFn = unsafe extern "system" fn(u32, u32) -> i32;
pub type GetComputerNameAFn = unsafe extern "system" fn(*mut c_char, *mut u32) -> i32;
pub type GetLocalTimeFn = unsafe extern "system" fn(*mut SystemTime);
pub type GetProcAddressFn = unsafe extern "system" fn(usize, *const c_char) -> *const c_void;
pub type GetSystemTimeAsFileTimeFn = unsafe extern "system" fn(*mut FileTime);
pub type GetSystemTimeFn = unsafe extern "system" fn(*mut SystemTime);
pub type GetTimeZoneInformationFn = unsafe extern "system" fn(*mut TimeZoneInformation) -> u32;
pub type LoadLibraryAFn = unsafe extern "system" fn(*const c_char) -> usize;
pub type LoadLibraryWFn = unsafe extern "system" fn(*const u16) -> usize;
pub type RegQueryValueExAFn =
    unsafe extern "system" fn(usize, *const c_char, *mut u32, *mut u32, *mut u8, *mut u32) -> u32;
pub type RegQueryValueExWFn =
    unsafe extern "system" fn(usize, *const u16, *mut u32, *mut u32, *mut u8, *mut u32) -> u32;
pub type SetLocalTimeFn = unsafe extern "system" fn(*const SystemTime) -> i32;
pub type SetSystemTimeFn = unsafe extern "system" fn(*const SystemTime) -> i32;
pub type SetTimeZoneInformationFn = unsafe extern "system" fn(*const TimeZoneInformation) -> i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FileTime {
    pub low_date_time: u32,
    pub high_date_time: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct TimeZoneInformation {
    pub bias: i32,
    pub standard_name: [u16; 32],
    pub standard_date: SystemTime,
    pub standard_bias: i32,
    pub daylight_name: [u16; 32],
    pub daylight_date: SystemTime,
    pub daylight_bias: i32,
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleA(module_name: *const u8) -> usize;
}

pub unsafe fn hook_import<T>(api: &Api, dll: &str, func: &str, detour: *const ()) -> Option<T>
where
    T: Copy,
{
    let original = hook_iat(api.game_base(), dll, func, detour)?;
    Some(std::mem::transmute_copy(&original))
}

pub fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    (!ptr.is_null()).then(|| {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    })
}

pub fn wide_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    Some(
        OsString::from_wide(unsafe { std::slice::from_raw_parts(ptr, len) })
            .to_string_lossy()
            .into_owned(),
    )
}

pub fn string_to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn to_cstring_lossy(value: &str) -> CString {
    let mut bytes = value.as_bytes().to_vec();
    bytes.retain(|byte| *byte != 0);
    CString::new(bytes).expect("nul bytes removed")
}

pub fn normalize_path(value: &str) -> String {
    value.replace('/', "\\").to_ascii_lowercase()
}

pub fn absolutize(base_dir: impl AsRef<Path>, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.as_ref().join(path)
    }
}

/// 将路径绝对化（相对路径用 CWD 解析，等同 GetFullPathNameW 行为）
/// 规范化 `.`/`..`、末尾保证一个反斜杠
/// 返回带尾部 `\` 的绝对路径字符串（全 `\` 分隔）
pub fn fixup_path(path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    // 规范化 . / ..（不触碰文件系统，纯词法，照 GetFullPathName 词法解析）
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other),
        }
    }

    let mut value = normalized.to_string_lossy().replace('/', "\\");
    if !value.ends_with('\\') {
        value.push('\\');
    }
    value
}

/// 查询 %USERPROFILE%，失败时回退 C:\Users\Default（极端兜底，避免 panic）
pub fn userprofile() -> String {
    std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Default".to_owned())
}

/// 逐级创建目录（已存在则跳过）
pub fn mkdir_rec(path: &str) {
    let _ = std::fs::create_dir_all(path);
}

pub fn clean_join(root: &Path, tail: &str) -> PathBuf {
    let mut result = root.to_path_buf();
    for component in Path::new(tail).components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            _ => {}
        }
    }
    result
}

pub unsafe fn write_reg_string(
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
    value: &str,
    wide: bool,
) -> u32 {
    if !data_type.is_null() {
        *data_type = REG_SZ;
    }

    let bytes = if wide {
        string_to_wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>()
    } else {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        bytes
    };
    write_reg_bytes(data, data_len, &bytes)
}

pub unsafe fn write_reg_dword(
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
    value: u32,
) -> u32 {
    if !data_type.is_null() {
        *data_type = REG_DWORD;
    }
    write_reg_bytes(data, data_len, &value.to_le_bytes())
}

pub unsafe fn write_reg_binary(
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
    value: &[u8],
) -> u32 {
    if !data_type.is_null() {
        *data_type = REG_BINARY;
    }
    write_reg_bytes(data, data_len, value)
}

unsafe fn write_reg_bytes(data: *mut u8, data_len: *mut u32, bytes: &[u8]) -> u32 {
    if !data_len.is_null() {
        let capacity = *data_len as usize;
        *data_len = bytes.len() as u32;
        if !data.is_null() && capacity < bytes.len() {
            return ERROR_INSUFFICIENT_BUFFER;
        }
    }

    if !data.is_null() {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
    }
    ERROR_SUCCESS
}
