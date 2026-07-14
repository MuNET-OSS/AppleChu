use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;

use retour::RawDetour;

struct HookEntry {
    detour: RawDetour,
}

unsafe impl Send for HookEntry {}
unsafe impl Sync for HookEntry {}

static HOOKS: once_cell::sync::Lazy<Mutex<HashMap<usize, HookEntry>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

pub unsafe extern "C" fn api_hook_create(
    target: *mut c_void,
    detour: *mut c_void,
    original: *mut *mut c_void,
) -> i32 {
    match RawDetour::new(target as *const (), detour as *const ()) {
        Ok(hook) => {
            let trampoline = hook.trampoline() as *const () as *mut c_void;
            if !original.is_null() {
                *original = trampoline;
            }
            let mut hooks = HOOKS.lock().unwrap();
            hooks.insert(target as usize, HookEntry { detour: hook });
            0
        }
        Err(_) => -1,
    }
}

pub unsafe extern "C" fn api_hook_enable(target: *mut c_void) -> i32 {
    let hooks = HOOKS.lock().unwrap();
    match hooks.get(&(target as usize)) {
        Some(entry) if entry.detour.enable().is_ok() => 0,
        _ => -1,
    }
}

pub unsafe extern "C" fn api_hook_disable(target: *mut c_void) -> i32 {
    let hooks = HOOKS.lock().unwrap();
    match hooks.get(&(target as usize)) {
        Some(entry) if entry.detour.disable().is_ok() => 0,
        _ => -1,
    }
}

pub unsafe extern "C" fn api_hook_remove(target: *mut c_void) -> i32 {
    let mut hooks = HOOKS.lock().unwrap();
    if hooks.remove(&(target as usize)).is_some() {
        0
    } else {
        -1
    }
}

pub fn shutdown() {
    let mut hooks = HOOKS.lock().unwrap();
    for (_addr, entry) in hooks.drain() {
        unsafe {
            let _ = entry.detour.disable();
        }
    }
}
