use std::ffi::{c_char, CStr};
use std::sync::Mutex;

static CURRENT_TOML_CONFIG: once_cell::sync::Lazy<Mutex<Option<toml::Value>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

pub fn load_current_config(path: Option<&str>) {
    let value = path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| text.parse::<toml::Value>().ok());
    if let Ok(mut current) = CURRENT_TOML_CONFIG.lock() {
        *current = value;
    }
}

unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

fn with_value<F, R>(f: F, default: R) -> R
where
    F: FnOnce(&toml::Value) -> R,
{
    match CURRENT_TOML_CONFIG.lock() {
        Ok(current) => current.as_ref().map_or(default, f),
        Err(_) => default,
    }
}

fn find_section<'a>(value: &'a toml::Value, section: &str) -> Option<&'a toml::Value> {
    let mut current = value;
    for part in section.split('.') {
        current = current.get(part)?;
    }
    current.as_table().map(|_| current)
}

fn find_key<'a>(value: &'a toml::Value, section: &str, key: &str) -> Option<&'a toml::Value> {
    find_section(value, section)?.get(key)
}

pub unsafe extern "C" fn api_toml_section_exists(section: *const c_char) -> i32 {
    let Some(section) = cstr_to_string(section) else {
        return 0;
    };
    with_value(
        |value| i32::from(find_section(value, &section).is_some()),
        0,
    )
}

pub unsafe extern "C" fn api_toml_get_bool(
    section: *const c_char,
    key: *const c_char,
    default: i32,
) -> i32 {
    let (Some(section), Some(key)) = (cstr_to_string(section), cstr_to_string(key)) else {
        return default;
    };
    with_value(
        |value| {
            find_key(value, &section, &key)
                .and_then(toml::Value::as_bool)
                .map_or(default, i32::from)
        },
        default,
    )
}

pub unsafe extern "C" fn api_toml_get_int(
    section: *const c_char,
    key: *const c_char,
    default: i32,
) -> i32 {
    let (Some(section), Some(key)) = (cstr_to_string(section), cstr_to_string(key)) else {
        return default;
    };
    with_value(
        |value| {
            find_key(value, &section, &key)
                .and_then(toml::Value::as_integer)
                .and_then(|v| i32::try_from(v).ok())
                .unwrap_or(default)
        },
        default,
    )
}

pub unsafe extern "C" fn api_toml_get_float(
    section: *const c_char,
    key: *const c_char,
    default: f32,
) -> f32 {
    let (Some(section), Some(key)) = (cstr_to_string(section), cstr_to_string(key)) else {
        return default;
    };
    with_value(
        |value| {
            find_key(value, &section, &key)
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
                .map_or(default, |v| v as f32)
        },
        default,
    )
}

pub unsafe extern "C" fn api_toml_get_string(
    section: *const c_char,
    key: *const c_char,
    buf: *mut c_char,
    buf_size: u32,
    default: *const c_char,
) -> i32 {
    if buf.is_null() || buf_size == 0 {
        return -1;
    }
    let (Some(section), Some(key)) = (cstr_to_string(section), cstr_to_string(key)) else {
        return -1;
    };
    let default_value = cstr_to_string(default).unwrap_or_default();
    let value = with_value(
        |root| {
            find_key(root, &section, &key)
                .and_then(toml::Value::as_str)
                .map_or_else(|| default_value.clone(), ToOwned::to_owned)
        },
        default_value.clone(),
    );
    let bytes = value.as_bytes();
    let copy_len = bytes.len().min(buf_size.saturating_sub(1) as usize);
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, copy_len);
    *(buf as *mut u8).add(copy_len) = 0;
    copy_len as i32
}
