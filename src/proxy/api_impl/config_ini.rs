use std::ffi::{c_char, CStr};
use std::sync::Mutex;

static CURRENT_CONFIG_PATH: once_cell::sync::Lazy<Mutex<String>> =
    once_cell::sync::Lazy::new(|| Mutex::new(String::new()));

extern "system" {
    fn GetPrivateProfileStringA(
        app: *const u8,
        key: *const u8,
        default: *const u8,
        ret: *mut u8,
        size: u32,
        file: *const u8,
    ) -> u32;
    fn GetPrivateProfileIntA(app: *const u8, key: *const u8, default: i32, file: *const u8) -> u32;
    fn WritePrivateProfileStringA(
        app: *const u8,
        key: *const u8,
        value: *const u8,
        file: *const u8,
    ) -> i32;
}

pub fn set_current_config(path: &str) {
    if let Ok(mut p) = CURRENT_CONFIG_PATH.lock() {
        *p = path.to_string();
    }
}

fn with_config_path<F, R>(f: F) -> R
where
    F: FnOnce(&str) -> R,
    R: Default,
{
    match CURRENT_CONFIG_PATH.lock() {
        Ok(p) if !p.is_empty() => f(&p),
        _ => R::default(),
    }
}

pub unsafe extern "C" fn api_config_get_int(key: *const c_char, default: i32) -> i32 {
    if key.is_null() {
        return default;
    }
    let key_str = CStr::from_ptr(key);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let file_c = format!("{}\0", path);
        GetPrivateProfileIntA(section.as_ptr(), key_c.as_ptr(), default, file_c.as_ptr()) as i32
    })
}

pub unsafe extern "C" fn api_config_get_float(key: *const c_char, default: f32) -> f32 {
    if key.is_null() {
        return default;
    }
    let key_str = CStr::from_ptr(key);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let file_c = format!("{}\0", path);
        let mut buf = [0u8; 64];
        let default_str = format!("{}\0", default);
        let len = GetPrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            default_str.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            file_c.as_ptr(),
        );
        if len == 0 {
            return default;
        }
        let s = std::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        s.parse::<f32>().unwrap_or(default)
    })
}

pub unsafe extern "C" fn api_config_get_bool(key: *const c_char, default: i32) -> i32 {
    if key.is_null() {
        return default;
    }
    let key_str = CStr::from_ptr(key);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let file_c = format!("{}\0", path);
        let mut buf = [0u8; 32];
        let default_str: &[u8] = if default != 0 { b"true\0" } else { b"false\0" };
        let len = GetPrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            default_str.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            file_c.as_ptr(),
        );
        if len == 0 {
            return default;
        }
        let s = std::str::from_utf8(&buf[..len as usize]).unwrap_or("");
        match s {
            "true" | "1" | "yes" => 1,
            "false" | "0" | "no" => 0,
            _ => default,
        }
    })
}

pub unsafe extern "C" fn api_config_get_string(
    key: *const c_char,
    buf: *mut c_char,
    buf_size: u32,
    default: *const c_char,
) -> i32 {
    if key.is_null() || buf.is_null() || buf_size == 0 {
        return -1;
    }
    let key_str = CStr::from_ptr(key);
    let default_c = if default.is_null() {
        b"\0".as_ptr()
    } else {
        default as *const u8
    };
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let file_c = format!("{}\0", path);
        let len = GetPrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            default_c,
            buf as *mut u8,
            buf_size,
            file_c.as_ptr(),
        );
        len as i32
    })
}

pub unsafe extern "C" fn api_config_set_int(key: *const c_char, value: i32) -> i32 {
    if key.is_null() {
        return -1;
    }
    let key_str = CStr::from_ptr(key);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let val_c = format!("{}\0", value);
        let file_c = format!("{}\0", path);
        WritePrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            val_c.as_ptr(),
            file_c.as_ptr(),
        ) - 1
    })
}

pub unsafe extern "C" fn api_config_set_float(key: *const c_char, value: f32) -> i32 {
    if key.is_null() {
        return -1;
    }
    let key_str = CStr::from_ptr(key);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let val_c = format!("{}\0", value);
        let file_c = format!("{}\0", path);
        WritePrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            val_c.as_ptr(),
            file_c.as_ptr(),
        ) - 1
    })
}

pub unsafe extern "C" fn api_config_set_bool(key: *const c_char, value: i32) -> i32 {
    if key.is_null() {
        return -1;
    }
    let key_str = CStr::from_ptr(key);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let val_c: &[u8] = if value != 0 { b"true\0" } else { b"false\0" };
        let file_c = format!("{}\0", path);
        WritePrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            val_c.as_ptr(),
            file_c.as_ptr(),
        ) - 1
    })
}

pub unsafe extern "C" fn api_config_set_string(key: *const c_char, value: *const c_char) -> i32 {
    if key.is_null() || value.is_null() {
        return -1;
    }
    let key_str = CStr::from_ptr(key);
    let val_str = CStr::from_ptr(value);
    with_config_path(|path| {
        let section = b"config\0";
        let key_c = format!("{}\0", key_str.to_string_lossy());
        let val_c = format!("{}\0", val_str.to_string_lossy());
        let file_c = format!("{}\0", path);
        WritePrivateProfileStringA(
            section.as_ptr(),
            key_c.as_ptr(),
            val_c.as_ptr(),
            file_c.as_ptr(),
        ) - 1
    })
}
