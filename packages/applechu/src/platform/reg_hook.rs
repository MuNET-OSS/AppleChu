use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::{Lazy, OnceCell};

use crate::config::Config;
use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use crate::iohook::proc_addr;
use crate::platform::winapi::{self, RegQueryValueExAFn, RegQueryValueExWFn};
use crate::util::api::Api;

pub const HKEY_CLASSES_ROOT: usize = 0x8000_0000;
pub const HKEY_CURRENT_USER: usize = 0x8000_0001;
pub const HKEY_LOCAL_MACHINE: usize = 0x8000_0002;
pub const HKEY_USERS: usize = 0x8000_0003;

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const REG_OPENED_EXISTING_KEY: u32 = 2;
const KEY_READ: u32 = 0x0002_0019;
const SOFTWARE_KEY: [u16; 9] = [
    b'S' as u16,
    b'O' as u16,
    b'F' as u16,
    b'T' as u16,
    b'W' as u16,
    b'A' as u16,
    b'R' as u16,
    b'E' as u16,
    0,
];

type RegOpenKeyExWFn = unsafe extern "system" fn(usize, *const u16, u32, u32, *mut usize) -> u32;
type RegCreateKeyExWFn = unsafe extern "system" fn(
    usize,
    *const u16,
    u32,
    *mut u16,
    u32,
    u32,
    *const c_void,
    *mut usize,
    *mut u32,
) -> u32;
type RegCloseKeyFn = unsafe extern "system" fn(usize) -> u32;
type RegGetValueWFn = unsafe extern "system" fn(
    usize,
    *const u16,
    *const u16,
    u32,
    *mut u32,
    *mut c_void,
    *mut u32,
) -> u32;
type RegQueryInfoKeyWFn = unsafe extern "system" fn(
    usize,
    *mut u16,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut c_void,
) -> u32;
type RegEnumValueWFn = unsafe extern "system" fn(
    usize,
    u32,
    *mut u16,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u8,
    *mut u32,
) -> u32;

static KEYS: Lazy<Mutex<Vec<VirtualKey>>> = Lazy::new(|| Mutex::new(Vec::new()));
static HANDLES: Lazy<Mutex<HashMap<usize, usize>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static API: OnceCell<Api> = OnceCell::new();

static ORIG_OPEN_KEY_EX_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_CREATE_KEY_EX_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_CLOSE_KEY: AtomicUsize = AtomicUsize::new(0);
static ORIG_QUERY_VALUE_EX_A: AtomicUsize = AtomicUsize::new(0);
static ORIG_QUERY_VALUE_EX_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_VALUE_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_QUERY_INFO_KEY_W: AtomicUsize = AtomicUsize::new(0);
static ORIG_ENUM_VALUE_W: AtomicUsize = AtomicUsize::new(0);

static mut ORIG_OPEN_KEY_EX_W_PTR: *const () = ptr::null();
static mut ORIG_CREATE_KEY_EX_W_PTR: *const () = ptr::null();
static mut ORIG_CLOSE_KEY_PTR: *const () = ptr::null();
static mut ORIG_QUERY_VALUE_EX_A_PTR: *const () = ptr::null();
static mut ORIG_QUERY_VALUE_EX_W_PTR: *const () = ptr::null();
static mut ORIG_GET_VALUE_W_PTR: *const () = ptr::null();
static mut ORIG_QUERY_INFO_KEY_W_PTR: *const () = ptr::null();
static mut ORIG_ENUM_VALUE_W_PTR: *const () = ptr::null();

#[derive(Clone)]
pub struct RegValue {
    name: String,
    ty: u32,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct VirtualKey {
    root: usize,
    path: String,
    values: Vec<RegValue>,
}

impl RegValue {
    pub fn binary(name: &str, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.to_owned(),
            ty: winapi::REG_BINARY,
            bytes: bytes.into(),
        }
    }

    pub fn dword(name: &str, value: u32) -> Self {
        Self {
            name: name.to_owned(),
            ty: winapi::REG_DWORD,
            bytes: value.to_le_bytes().to_vec(),
        }
    }

    pub fn string(name: &str, value: &str) -> Self {
        let bytes = winapi::string_to_wide(value)
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();
        Self {
            name: name.to_owned(),
            ty: winapi::REG_SZ,
            bytes,
        }
    }
}

#[applechu_macros::config_section(stage = Platform, order = 10)]
pub fn init(api: &Api, _config: &Config) {
    let _ = API.set(*api);
    install_hooks(api);
    push_key(HKEY_LOCAL_MACHINE, "SYSTEM\\SEGA", Vec::new());
    push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty",
        Vec::new(),
    );
}

pub fn push_key(root: usize, path: &str, values: Vec<RegValue>) {
    let key = VirtualKey {
        root,
        path: canonical_path(path),
        values,
    };
    let value_count = key.values.len();
    if let Ok(mut keys) = KEYS.lock() {
        if let Some(existing) = keys.iter_mut().find(|existing| {
            existing.root == key.root && existing.path.eq_ignore_ascii_case(&key.path)
        }) {
            existing.values = key.values;
        } else {
            keys.push(key.clone());
        }
    }
    if let Some(api) = API.get() {
        api.log_info(&format!(
            "reg_hook: registered HKLM\\{} ({value_count} values)",
            key.path
        ));
    }
}

fn install_hooks(api: &Api) {
    unsafe {
        let symbols = [
            HookSymbol {
                name: "RegOpenKeyExW",
                patch: hooked_reg_open_key_ex_w as *const (),
                original: ptr::addr_of_mut!(ORIG_OPEN_KEY_EX_W_PTR),
            },
            HookSymbol {
                name: "RegCreateKeyExW",
                patch: hooked_reg_create_key_ex_w as *const (),
                original: ptr::addr_of_mut!(ORIG_CREATE_KEY_EX_W_PTR),
            },
            HookSymbol {
                name: "RegCloseKey",
                patch: hooked_reg_close_key as *const (),
                original: ptr::addr_of_mut!(ORIG_CLOSE_KEY_PTR),
            },
            HookSymbol {
                name: "RegQueryValueExA",
                patch: hooked_reg_query_value_ex_a as *const (),
                original: ptr::addr_of_mut!(ORIG_QUERY_VALUE_EX_A_PTR),
            },
            HookSymbol {
                name: "RegQueryValueExW",
                patch: hooked_reg_query_value_ex_w as *const (),
                original: ptr::addr_of_mut!(ORIG_QUERY_VALUE_EX_W_PTR),
            },
            HookSymbol {
                name: "RegGetValueW",
                patch: hooked_reg_get_value_w as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_VALUE_W_PTR),
            },
            HookSymbol {
                name: "RegQueryInfoKeyW",
                patch: hooked_reg_query_info_key_w as *const (),
                original: ptr::addr_of_mut!(ORIG_QUERY_INFO_KEY_W_PTR),
            },
            HookSymbol {
                name: "RegEnumValueW",
                patch: hooked_reg_enum_value_w as *const (),
                original: ptr::addr_of_mut!(ORIG_ENUM_VALUE_W_PTR),
            },
        ];
        proc_addr::push("advapi32.dll", &symbols, sync_originals);
        let patched = hook_table_apply(null_module(), "advapi32.dll", &symbols);
        sync_originals();
        if patched > 0 {
            api.log_info(&format!("reg_hook: advapi32 hooks installed ({patched})"));
        }
    }
}

fn sync_originals() {
    // SAFETY: 原函数槽由 hook 安装过程写入，随后通过原子变量发布给并发调用者。
    unsafe {
        ORIG_OPEN_KEY_EX_W.store(ORIG_OPEN_KEY_EX_W_PTR as usize, Ordering::SeqCst);
        ORIG_CREATE_KEY_EX_W.store(ORIG_CREATE_KEY_EX_W_PTR as usize, Ordering::SeqCst);
        ORIG_CLOSE_KEY.store(ORIG_CLOSE_KEY_PTR as usize, Ordering::SeqCst);
        ORIG_QUERY_VALUE_EX_A.store(ORIG_QUERY_VALUE_EX_A_PTR as usize, Ordering::SeqCst);
        ORIG_QUERY_VALUE_EX_W.store(ORIG_QUERY_VALUE_EX_W_PTR as usize, Ordering::SeqCst);
        ORIG_GET_VALUE_W.store(ORIG_GET_VALUE_W_PTR as usize, Ordering::SeqCst);
        ORIG_QUERY_INFO_KEY_W.store(ORIG_QUERY_INFO_KEY_W_PTR as usize, Ordering::SeqCst);
        ORIG_ENUM_VALUE_W.store(ORIG_ENUM_VALUE_W_PTR as usize, Ordering::SeqCst);
    }
}

unsafe extern "system" fn hooked_reg_open_key_ex_w(
    parent: usize,
    subkey: *const u16,
    options: u32,
    access: u32,
    out: *mut usize,
) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if let Some(key_id) = resolve_key_id(parent, subkey) {
        return open_virtual_key(key_id, out);
    }
    original_open_key_ex_w(parent, subkey, options, access, out)
}

unsafe extern "system" fn hooked_reg_create_key_ex_w(
    parent: usize,
    subkey: *const u16,
    reserved: u32,
    class: *mut u16,
    options: u32,
    access: u32,
    security_attributes: *const c_void,
    out: *mut usize,
    disposition: *mut u32,
) -> u32 {
    if out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if let Some(key_id) = resolve_key_id(parent, subkey) {
        let result = open_virtual_key(key_id, out);
        if result == winapi::ERROR_SUCCESS && !disposition.is_null() {
            *disposition = REG_OPENED_EXISTING_KEY;
        }
        return result;
    }
    original_create_key_ex_w(
        parent,
        subkey,
        reserved,
        class,
        options,
        access,
        security_attributes,
        out,
        disposition,
    )
}

unsafe extern "system" fn hooked_reg_close_key(handle: usize) -> u32 {
    if let Ok(mut handles) = HANDLES.lock() {
        handles.remove(&handle);
    }
    original_close_key(handle)
}

unsafe extern "system" fn hooked_reg_query_value_ex_a(
    handle: usize,
    name: *const c_char,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    if let Some(key) = key_for_handle(handle) {
        let name = winapi::cstr_to_string(name).unwrap_or_default();
        return query_value(&key, &name, data_type, data, data_len);
    }
    original_query_value_ex_a(handle, name, reserved, data_type, data, data_len)
}

unsafe extern "system" fn hooked_reg_query_value_ex_w(
    handle: usize,
    name: *const u16,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    if let Some(key) = key_for_handle(handle) {
        let name = winapi::wide_to_string(name).unwrap_or_default();
        return query_value(&key, &name, data_type, data, data_len);
    }
    original_query_value_ex_w(handle, name, reserved, data_type, data, data_len)
}

unsafe extern "system" fn hooked_reg_get_value_w(
    handle: usize,
    subkey: *const u16,
    name: *const u16,
    flags: u32,
    data_type: *mut u32,
    data: *mut c_void,
    data_len: *mut u32,
) -> u32 {
    if let Some(key_id) = resolve_key_id(handle, subkey) {
        if let Some(key) = key_by_id(key_id) {
            let name = winapi::wide_to_string(name).unwrap_or_default();
            return query_value(&key, &name, data_type, data.cast::<u8>(), data_len);
        }
    }
    original_get_value_w(handle, subkey, name, flags, data_type, data, data_len)
}

unsafe extern "system" fn hooked_reg_query_info_key_w(
    handle: usize,
    class: *mut u16,
    class_len: *mut u32,
    reserved: *mut u32,
    subkeys: *mut u32,
    max_subkey_len: *mut u32,
    max_class_len: *mut u32,
    values: *mut u32,
    max_value_name_len: *mut u32,
    max_value_len: *mut u32,
    security_descriptor: *mut u32,
    last_write_time: *mut c_void,
) -> u32 {
    if let Some(key) = key_for_handle(handle) {
        if !values.is_null() {
            *values = key.values.len() as u32;
        }
        if !max_value_name_len.is_null() {
            *max_value_name_len = key
                .values
                .iter()
                .map(|value| value.name.encode_utf16().count())
                .max()
                .unwrap_or(0) as u32;
        }
        if !max_value_len.is_null() {
            *max_value_len = key
                .values
                .iter()
                .map(|value| value.bytes.len())
                .max()
                .unwrap_or(0) as u32;
        }
        if !subkeys.is_null() {
            *subkeys = 0;
        }
        return winapi::ERROR_SUCCESS;
    }
    original_query_info_key_w(
        handle,
        class,
        class_len,
        reserved,
        subkeys,
        max_subkey_len,
        max_class_len,
        values,
        max_value_name_len,
        max_value_len,
        security_descriptor,
        last_write_time,
    )
}

unsafe extern "system" fn hooked_reg_enum_value_w(
    handle: usize,
    index: u32,
    value_name: *mut u16,
    value_name_len: *mut u32,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    if let Some(key) = key_for_handle(handle) {
        let Some(value) = key.values.get(index as usize) else {
            return ERROR_NO_MORE_ITEMS;
        };
        let name = winapi::string_to_wide(&value.name);
        let name_len = name.len() as u32 - 1;
        if !value_name_len.is_null() {
            let capacity = *value_name_len;
            *value_name_len = name_len;
            if !value_name.is_null() {
                if capacity <= name_len {
                    return ERROR_MORE_DATA;
                }
                ptr::copy_nonoverlapping(name.as_ptr(), value_name, name.len());
            }
        }
        return write_value(value, data_type, data, data_len);
    }
    original_enum_value_w(
        handle,
        index,
        value_name,
        value_name_len,
        reserved,
        data_type,
        data,
        data_len,
    )
}

unsafe fn open_virtual_key(key_id: usize, out: *mut usize) -> u32 {
    let mut token = 0usize;
    let result = original_open_key_ex_w(
        HKEY_LOCAL_MACHINE,
        SOFTWARE_KEY.as_ptr(),
        0,
        KEY_READ,
        &mut token,
    );
    if result == winapi::ERROR_SUCCESS {
        if let Ok(mut handles) = HANDLES.lock() {
            handles.insert(token, key_id);
        }
        *out = token;
    }
    result
}

unsafe fn query_value(
    key: &VirtualKey,
    name: &str,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    let Some(value) = key
        .values
        .iter()
        .find(|value| value.name.eq_ignore_ascii_case(name))
    else {
        return winapi::ERROR_FILE_NOT_FOUND;
    };
    write_value(value, data_type, data, data_len)
}

unsafe fn write_value(
    value: &RegValue,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    if !data_type.is_null() {
        *data_type = value.ty;
    }
    if !data_len.is_null() {
        let capacity = *data_len as usize;
        *data_len = value.bytes.len() as u32;
        if !data.is_null() && capacity < value.bytes.len() {
            return ERROR_MORE_DATA;
        }
    } else if !data.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    if !data.is_null() && !value.bytes.is_empty() {
        ptr::copy_nonoverlapping(value.bytes.as_ptr(), data, value.bytes.len());
    }
    winapi::ERROR_SUCCESS
}

fn resolve_key_id(parent: usize, subkey: *const u16) -> Option<usize> {
    let subkey = winapi::wide_to_string(subkey).unwrap_or_default();
    let (root, path) = resolve_candidate(parent, &subkey)?;
    KEYS.lock()
        .ok()?
        .iter()
        .position(|key| key.root == root && key.path.eq_ignore_ascii_case(&path))
}

fn resolve_candidate(parent: usize, subkey: &str) -> Option<(usize, String)> {
    let subkey = canonical_path(subkey);
    if is_predefined_root(parent) {
        return Some((parent, subkey));
    }

    let key = key_for_handle(parent)?;
    if subkey.is_empty() {
        Some((key.root, key.path))
    } else if key.path.is_empty() {
        Some((key.root, subkey))
    } else {
        Some((key.root, format!("{}\\{}", key.path, subkey)))
    }
}

fn key_for_handle(handle: usize) -> Option<VirtualKey> {
    let key_id = *HANDLES.lock().ok()?.get(&handle)?;
    key_by_id(key_id)
}

fn key_by_id(key_id: usize) -> Option<VirtualKey> {
    KEYS.lock().ok()?.get(key_id).cloned()
}

fn canonical_path(path: &str) -> String {
    path.replace('/', "\\").trim_matches('\\').to_owned()
}

fn is_predefined_root(handle: usize) -> bool {
    matches!(
        handle,
        HKEY_CLASSES_ROOT | HKEY_CURRENT_USER | HKEY_LOCAL_MACHINE | HKEY_USERS
    )
}

unsafe fn original_open_key_ex_w(
    parent: usize,
    subkey: *const u16,
    options: u32,
    access: u32,
    out: *mut usize,
) -> u32 {
    let original_addr = ORIG_OPEN_KEY_EX_W.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegOpenKeyExWFn = std::mem::transmute(original_addr);
    original(parent, subkey, options, access, out)
}

unsafe fn original_create_key_ex_w(
    parent: usize,
    subkey: *const u16,
    reserved: u32,
    class: *mut u16,
    options: u32,
    access: u32,
    security_attributes: *const c_void,
    out: *mut usize,
    disposition: *mut u32,
) -> u32 {
    let original_addr = ORIG_CREATE_KEY_EX_W.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegCreateKeyExWFn = std::mem::transmute(original_addr);
    original(
        parent,
        subkey,
        reserved,
        class,
        options,
        access,
        security_attributes,
        out,
        disposition,
    )
}

unsafe fn original_close_key(handle: usize) -> u32 {
    let original_addr = ORIG_CLOSE_KEY.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_SUCCESS;
    }
    let original: RegCloseKeyFn = std::mem::transmute(original_addr);
    original(handle)
}

unsafe fn original_query_value_ex_a(
    handle: usize,
    name: *const c_char,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    let original_addr = ORIG_QUERY_VALUE_EX_A.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegQueryValueExAFn = std::mem::transmute(original_addr);
    original(handle, name, reserved, data_type, data, data_len)
}

unsafe fn original_query_value_ex_w(
    handle: usize,
    name: *const u16,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    let original_addr = ORIG_QUERY_VALUE_EX_W.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegQueryValueExWFn = std::mem::transmute(original_addr);
    original(handle, name, reserved, data_type, data, data_len)
}

unsafe fn original_get_value_w(
    handle: usize,
    subkey: *const u16,
    name: *const u16,
    flags: u32,
    data_type: *mut u32,
    data: *mut c_void,
    data_len: *mut u32,
) -> u32 {
    let original_addr = ORIG_GET_VALUE_W.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegGetValueWFn = std::mem::transmute(original_addr);
    original(handle, subkey, name, flags, data_type, data, data_len)
}

unsafe fn original_query_info_key_w(
    handle: usize,
    class: *mut u16,
    class_len: *mut u32,
    reserved: *mut u32,
    subkeys: *mut u32,
    max_subkey_len: *mut u32,
    max_class_len: *mut u32,
    values: *mut u32,
    max_value_name_len: *mut u32,
    max_value_len: *mut u32,
    security_descriptor: *mut u32,
    last_write_time: *mut c_void,
) -> u32 {
    let original_addr = ORIG_QUERY_INFO_KEY_W.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegQueryInfoKeyWFn = std::mem::transmute(original_addr);
    original(
        handle,
        class,
        class_len,
        reserved,
        subkeys,
        max_subkey_len,
        max_class_len,
        values,
        max_value_name_len,
        max_value_len,
        security_descriptor,
        last_write_time,
    )
}

unsafe fn original_enum_value_w(
    handle: usize,
    index: u32,
    value_name: *mut u16,
    value_name_len: *mut u32,
    reserved: *mut u32,
    data_type: *mut u32,
    data: *mut u8,
    data_len: *mut u32,
) -> u32 {
    let original_addr = ORIG_ENUM_VALUE_W.load(Ordering::SeqCst);
    if original_addr == 0 {
        return winapi::ERROR_FILE_NOT_FOUND;
    }
    let original: RegEnumValueWFn = std::mem::transmute(original_addr);
    original(
        handle,
        index,
        value_name,
        value_name_len,
        reserved,
        data_type,
        data,
        data_len,
    )
}
