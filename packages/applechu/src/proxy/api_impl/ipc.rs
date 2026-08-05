use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

static SERVICES: once_cell::sync::Lazy<Mutex<HashMap<String, SendPtr>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

struct Subscriber {
    topic: String,
    cb: unsafe extern "C" fn(*const c_char, *mut c_void, u32),
}

unsafe impl Send for Subscriber {}

static SUBSCRIBERS: once_cell::sync::Lazy<Mutex<Vec<Subscriber>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(Vec::new()));

pub unsafe extern "C" fn api_register_service(name: *const c_char, ptr: *mut c_void) -> i32 {
    if name.is_null() {
        return -1;
    }
    let key = CStr::from_ptr(name).to_string_lossy().into_owned();
    let mut services = SERVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    services.insert(key, SendPtr(ptr));
    0
}

pub unsafe extern "C" fn api_get_service(name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let key = CStr::from_ptr(name).to_string_lossy();
    let services = SERVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    services
        .get(key.as_ref())
        .map_or(std::ptr::null_mut(), |p| p.0)
}

pub unsafe extern "C" fn api_publish(topic: *const c_char, data: *mut c_void, size: u32) -> i32 {
    if topic.is_null() {
        return -1;
    }
    let topic_str = CStr::from_ptr(topic).to_string_lossy();
    let subs = SUBSCRIBERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for sub in subs.iter() {
        if sub.topic == topic_str.as_ref() {
            (sub.cb)(topic, data, size);
        }
    }
    0
}

pub unsafe extern "C" fn api_subscribe(
    topic: *const c_char,
    callback: Option<unsafe extern "C" fn(*const c_char, *mut c_void, u32)>,
) -> i32 {
    if topic.is_null() {
        return -1;
    }
    let cb = match callback {
        Some(f) => f,
        None => return -1,
    };
    let topic_str = CStr::from_ptr(topic).to_string_lossy().into_owned();
    let mut subs = SUBSCRIBERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    subs.push(Subscriber {
        topic: topic_str,
        cb,
    });
    0
}

pub fn shutdown() {
    SERVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    SUBSCRIBERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}
