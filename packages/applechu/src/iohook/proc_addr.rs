use std::cell::Cell;
use std::ffi::{CStr, c_char};
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::Lazy;
use windows_sys::Win32::Foundation::{HANDLE, HMODULE};

use crate::iohook::hook_table::{self, HookSymbol};
use crate::util::api::Api;

type GetProcAddressFn = unsafe extern "system" fn(usize, *const u8) -> *const ();
type LoadLibraryAFn = unsafe extern "system" fn(*const c_char) -> usize;
type LoadLibraryWFn = unsafe extern "system" fn(*const u16) -> usize;
type LoadLibraryExAFn = unsafe extern "system" fn(*const c_char, HANDLE, u32) -> usize;
type LoadLibraryExWFn = unsafe extern "system" fn(*const u16, HANDLE, u32) -> usize;

pub type GpaOverride = fn(usize, &str) -> Option<*const ()>;
pub type LoadOverrideA = fn(&str) -> Option<usize>;
pub type LoadOverrideW = fn(&str) -> Option<usize>;

#[derive(Clone)]
struct ProcAddrSymbol {
    name: &'static str,
    patch: usize,
    original: usize,
}

#[derive(Clone)]
struct ProcAddrTable {
    dll_name: &'static str,
    symbols: Vec<ProcAddrSymbol>,
}

static TABLES: Lazy<Mutex<Vec<ProcAddrTable>>> = Lazy::new(|| Mutex::new(Vec::new()));
static GPA_OVERRIDES: Lazy<Mutex<Vec<GpaOverride>>> = Lazy::new(|| Mutex::new(Vec::new()));
static LOAD_OVERRIDES_A: Lazy<Mutex<Vec<LoadOverrideA>>> = Lazy::new(|| Mutex::new(Vec::new()));
static LOAD_OVERRIDES_W: Lazy<Mutex<Vec<LoadOverrideW>>> = Lazy::new(|| Mutex::new(Vec::new()));

static REAL_GET_PROC_ADDRESS: AtomicUsize = AtomicUsize::new(0);

static mut ORIG_GET_PROC_ADDRESS_PTR: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_A_PTR: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_W_PTR: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_EX_A_PTR: *const () = ptr::null();
static mut ORIG_LOAD_LIBRARY_EX_W_PTR: *const () = ptr::null();

thread_local! {
    static REENTER: Cell<bool> = const { Cell::new(false) };
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleA(module_name: *const u8) -> HMODULE;
    fn GetModuleHandleW(module_name: *const u16) -> HMODULE;
    fn GetProcAddress(module: HMODULE, proc_name: *const u8) -> *const ();
}

struct ReentryGuard;

impl ReentryGuard {
    fn enter() -> Self {
        REENTER.with(|flag| flag.set(true));
        Self
    }
}

impl Drop for ReentryGuard {
    fn drop(&mut self) {
        REENTER.with(|flag| flag.set(false));
    }
}

#[applechu_macros::config_section(stage = PlatformCore, order = 10)]
pub fn init(api: &Api) {
    unsafe {
        cache_real_get_proc_address();

        let symbols = [
            HookSymbol {
                name: "GetProcAddress",
                patch: hooked_get_proc_address as *const (),
                original: ptr::addr_of_mut!(ORIG_GET_PROC_ADDRESS_PTR),
            },
            HookSymbol {
                name: "LoadLibraryA",
                patch: hooked_load_library_a as *const (),
                original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_A_PTR),
            },
            HookSymbol {
                name: "LoadLibraryW",
                patch: hooked_load_library_w as *const (),
                original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_W_PTR),
            },
            HookSymbol {
                name: "LoadLibraryExA",
                patch: hooked_load_library_ex_a as *const (),
                original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_EX_A_PTR),
            },
            HookSymbol {
                name: "LoadLibraryExW",
                patch: hooked_load_library_ex_w as *const (),
                original: ptr::addr_of_mut!(ORIG_LOAD_LIBRARY_EX_W_PTR),
            },
        ];
        let patched =
            hook_table::hook_table_apply(hook_table::null_module(), "kernel32.dll", &symbols);
        api.log_info(&format!("proc_addr: kernel32 hooks installed ({patched})"));
    }
}

pub fn push(dll_name: &'static str, symbols: &[HookSymbol]) {
    let entries = symbols
        .iter()
        .map(|symbol| ProcAddrSymbol {
            name: symbol.name,
            patch: symbol.patch as usize,
            original: symbol.original as usize,
        })
        .collect::<Vec<_>>();
    let count = entries.len();
    if let Ok(mut tables) = TABLES.lock() {
        tables.push(ProcAddrTable {
            dll_name,
            symbols: entries,
        });
    }
    log_info(&format!(
        "proc_addr: registered {dll_name} ({count} symbols)"
    ));
}

pub fn push_get_proc_override(_dll_name: &'static str, handler: GpaOverride) {
    if let Ok(mut handlers) = GPA_OVERRIDES.lock() {
        handlers.push(handler);
    }
    log_info("proc_addr: registered GetProcAddress override");
}

pub fn push_load_override_a(handler: LoadOverrideA) {
    if let Ok(mut handlers) = LOAD_OVERRIDES_A.lock() {
        handlers.push(handler);
    }
    log_info("proc_addr: registered LoadLibraryA override");
}

pub fn push_load_override_w(handler: LoadOverrideW) {
    if let Ok(mut handlers) = LOAD_OVERRIDES_W.lock() {
        handlers.push(handler);
    }
    log_info("proc_addr: registered LoadLibraryW override");
}

pub fn rehook_module(module: HMODULE) -> usize {
    let tables = snapshot_tables();
    tables
        .iter()
        .map(|table| unsafe {
            hook_table::hook_table_apply(module, table.dll_name, &table.to_hook_symbols())
        })
        .sum()
}

unsafe extern "system" fn hooked_get_proc_address(
    module: usize,
    proc_name: *const u8,
) -> *const () {
    if module == 0 || proc_name.is_null() || (proc_name as usize) <= 0xFFFF {
        return real_gpa(module, proc_name);
    }
    if REENTER.with(Cell::get) {
        return real_gpa(module, proc_name);
    }

    let _guard = ReentryGuard::enter();
    let real = real_gpa(module, proc_name);
    let name = CStr::from_ptr(proc_name.cast());
    if let Some(detour) = lookup_detour(module, name, real) {
        return detour;
    }
    if let Some(detour) = lookup_override(module, name) {
        return detour;
    }
    real
}

unsafe extern "system" fn hooked_load_library_a(name: *const c_char) -> usize {
    if REENTER.with(Cell::get) {
        return original_load_library_a(name);
    }

    let _guard = ReentryGuard::enter();
    if let Some(module) = lookup_load_override_a(name) {
        return module;
    }
    let module = original_load_library_a(name);
    if module != 0 {
        rehook_module(crate::util::win32::handle_from_value(module));
    }
    module
}

unsafe extern "system" fn hooked_load_library_w(name: *const u16) -> usize {
    if REENTER.with(Cell::get) {
        return original_load_library_w(name);
    }

    let _guard = ReentryGuard::enter();
    if let Some(module) = lookup_load_override_w(name) {
        return module;
    }
    let module = original_load_library_w(name);
    if module != 0 {
        rehook_module(crate::util::win32::handle_from_value(module));
    }
    module
}

unsafe extern "system" fn hooked_load_library_ex_a(
    name: *const c_char,
    file: HANDLE,
    flags: u32,
) -> usize {
    if REENTER.with(Cell::get) {
        return original_load_library_ex_a(name, file, flags);
    }

    let _guard = ReentryGuard::enter();
    if let Some(module) = lookup_load_override_a(name) {
        return module;
    }
    let module = original_load_library_ex_a(name, file, flags);
    if module != 0 {
        rehook_module(crate::util::win32::handle_from_value(module));
    }
    module
}

unsafe extern "system" fn hooked_load_library_ex_w(
    name: *const u16,
    file: HANDLE,
    flags: u32,
) -> usize {
    if REENTER.with(Cell::get) {
        return original_load_library_ex_w(name, file, flags);
    }

    let _guard = ReentryGuard::enter();
    if let Some(module) = lookup_load_override_w(name) {
        return module;
    }
    let module = original_load_library_ex_w(name, file, flags);
    if module != 0 {
        rehook_module(crate::util::win32::handle_from_value(module));
    }
    module
}

fn lookup_detour(module: usize, name: &CStr, real: *const ()) -> Option<*const ()> {
    let tables = snapshot_tables();
    for table in tables {
        let dll_module = unsafe { GetModuleHandleA(c_name(table.dll_name).as_ptr()) };
        if dll_module as usize == module {
            if let Some(symbol) = table
                .symbols
                .iter()
                .find(|symbol| name.to_bytes() == symbol.name.as_bytes())
            {
                unsafe { write_original(symbol.original, real) };
                return Some(symbol.patch as *const ());
            }
        }
    }
    None
}

fn lookup_override(module: usize, name: &CStr) -> Option<*const ()> {
    let name = name.to_str().ok()?;
    let handlers = GPA_OVERRIDES
        .lock()
        .ok()
        .map(|handlers| handlers.clone())
        .unwrap_or_default();
    handlers
        .into_iter()
        .find_map(|handler| handler(module, name))
}

unsafe fn lookup_load_override_a(name: *const c_char) -> Option<usize> {
    let name = cstr_to_string(name)?;
    let handlers = LOAD_OVERRIDES_A
        .lock()
        .ok()
        .map(|handlers| handlers.clone())
        .unwrap_or_default();
    handlers.into_iter().find_map(|handler| handler(&name))
}

unsafe fn lookup_load_override_w(name: *const u16) -> Option<usize> {
    let name = wide_to_string(name)?;
    let handlers = LOAD_OVERRIDES_W
        .lock()
        .ok()
        .map(|handlers| handlers.clone())
        .unwrap_or_default();
    handlers.into_iter().find_map(|handler| handler(&name))
}

fn snapshot_tables() -> Vec<ProcAddrTable> {
    TABLES
        .lock()
        .ok()
        .map(|tables| tables.clone())
        .unwrap_or_default()
}

impl ProcAddrTable {
    fn to_hook_symbols(&self) -> Vec<HookSymbol> {
        self.symbols
            .iter()
            .map(|symbol| HookSymbol {
                name: symbol.name,
                patch: symbol.patch as *const (),
                original: ptr::null_mut(),
            })
            .collect()
    }
}

unsafe fn write_original(original: usize, real: *const ()) {
    if original == 0 || real.is_null() {
        return;
    }

    let slot = original as *mut *const ();
    if !slot.is_null() && (*slot).is_null() {
        *slot = real;
    }
}

unsafe fn cache_real_get_proc_address() {
    if REAL_GET_PROC_ADDRESS.load(Ordering::SeqCst) != 0 {
        return;
    }

    static KERNEL32: &[u16] = &[
        b'k' as u16,
        b'e' as u16,
        b'r' as u16,
        b'n' as u16,
        b'e' as u16,
        b'l' as u16,
        b'3' as u16,
        b'2' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    let kernel32 = GetModuleHandleW(KERNEL32.as_ptr());
    let proc = GetProcAddress(kernel32, b"GetProcAddress\0".as_ptr());
    REAL_GET_PROC_ADDRESS.store(proc as usize, Ordering::SeqCst);
}

unsafe fn real_gpa(module: usize, proc_name: *const u8) -> *const () {
    let addr = REAL_GET_PROC_ADDRESS.load(Ordering::SeqCst);
    if addr == 0 {
        return ptr::null();
    }

    let original: GetProcAddressFn = std::mem::transmute(addr);
    original(module, proc_name)
}

unsafe fn original_load_library_a(name: *const c_char) -> usize {
    if ORIG_LOAD_LIBRARY_A_PTR.is_null() {
        return real_load_via_w_fallback_a(name);
    }
    let original: LoadLibraryAFn = std::mem::transmute(ORIG_LOAD_LIBRARY_A_PTR);
    original(name)
}

unsafe fn real_load_via_w_fallback_a(name: *const c_char) -> usize {
    let Some(name) = cstr_to_string(name) else {
        return 0;
    };
    let wide = name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    original_load_library_w(wide.as_ptr())
}

unsafe fn original_load_library_w(name: *const u16) -> usize {
    if ORIG_LOAD_LIBRARY_W_PTR.is_null() {
        let kernel32 = GetModuleHandleW(
            [
                b'k' as u16,
                b'e' as u16,
                b'r' as u16,
                b'n' as u16,
                b'e' as u16,
                b'l' as u16,
                b'3' as u16,
                b'2' as u16,
                b'.' as u16,
                b'd' as u16,
                b'l' as u16,
                b'l' as u16,
                0,
            ]
            .as_ptr(),
        );
        let proc = GetProcAddress(kernel32, b"LoadLibraryW\0".as_ptr());
        if proc.is_null() {
            return 0;
        }
        let original: LoadLibraryWFn = std::mem::transmute(proc);
        return original(name);
    }
    let original: LoadLibraryWFn = std::mem::transmute(ORIG_LOAD_LIBRARY_W_PTR);
    original(name)
}

unsafe fn original_load_library_ex_a(name: *const c_char, file: HANDLE, flags: u32) -> usize {
    if ORIG_LOAD_LIBRARY_EX_A_PTR.is_null() {
        return original_load_library_ex_w_fallback_a(name, file, flags);
    }
    let original: LoadLibraryExAFn = std::mem::transmute(ORIG_LOAD_LIBRARY_EX_A_PTR);
    original(name, file, flags)
}

unsafe fn original_load_library_ex_w_fallback_a(
    name: *const c_char,
    file: HANDLE,
    flags: u32,
) -> usize {
    let Some(name) = cstr_to_string(name) else {
        return 0;
    };
    let wide = name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    original_load_library_ex_w(wide.as_ptr(), file, flags)
}

unsafe fn original_load_library_ex_w(name: *const u16, file: HANDLE, flags: u32) -> usize {
    if ORIG_LOAD_LIBRARY_EX_W_PTR.is_null() {
        return original_load_library_w(name);
    }
    let original: LoadLibraryExWFn = std::mem::transmute(ORIG_LOAD_LIBRARY_EX_W_PTR);
    original(name, file, flags)
}

fn c_name(name: &str) -> Vec<u8> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

unsafe fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    (!ptr.is_null()).then(|| CStr::from_ptr(ptr).to_string_lossy().into_owned())
}

unsafe fn wide_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(
        ptr, len,
    )))
}

fn log_info(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}
