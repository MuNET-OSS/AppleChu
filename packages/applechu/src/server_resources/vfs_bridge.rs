//! Option 枚举注入
//!
//! 服务器下发的自定义 option 解压在缓存目录里，由 `platform/vfs` 把
//! `c:\mount\option\<AXXX>\...` 重定向到那里——**读取与子目录枚举都交给游戏走
//! 真实文件 API**。唯独 option 根那一级枚举需要本模块补一刀：包内根目录并不在
//! 游戏真实的 option 目录里，`FindFirstFile` 扫 `c:\mount\option\*` 时看不见它。
//!
//! 语义是**叠加**：真实枚举照常跑并原样返回，虚拟根追加在其后，磁盘原有
//! option（A001 等）的顺序与内容完全不变。
//!
//! 游戏 option 扫描走的是 **ANSI**（`FindFirstFileExA` + `FindNextFileA`），
//! 因此 A 与 W 两个变体都必须挂。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

use crate::platform::{vfs, winapi};
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

const KERNEL32_DLL: &str = "kernel32.dll";
/// 合成枚举句柄的高位标记
const SYNTHETIC_FLAG: usize = 1 << 31;
/// 同时打开的合成枚举上限，避免异常调用把状态表撑爆
const MAX_SYNTHETIC: usize = 64;
/// `WIN32_FIND_DATAA` / `WIN32_FIND_DATAW` 的名称都在偏移 44，
/// 宽度分别是 260 字节与 260 个 UTF-16 单元
const FIND_DATA_NAME_OFFSET: usize = 44;
const FIND_DATA_NAME_MAX: usize = 260;
const FIND_DATA_SIZE_A: usize = 320;
const FIND_DATA_SIZE_W: usize = 592;

type FindFirstFileAFn = unsafe extern "system" fn(*const i8, *mut c_void) -> HANDLE;
type FindFirstFileWFn = unsafe extern "system" fn(*const u16, *mut c_void) -> HANDLE;
type FindFirstFileExAFn =
    unsafe extern "system" fn(*const i8, u32, *mut c_void, u32, *mut c_void, u32) -> HANDLE;
type FindFirstFileExWFn =
    unsafe extern "system" fn(*const u16, u32, *mut c_void, u32, *mut c_void, u32) -> HANDLE;
type FindNextFileFn = unsafe extern "system" fn(HANDLE, *mut c_void) -> i32;
type FindCloseFn = unsafe extern "system" fn(HANDLE) -> i32;

/// 包装后的 API 指针（未命中虚拟内容时转交 path_hook 的下一跳）
static NEXT_FIND_FIRST_FILE_A: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_FIRST_FILE_W: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_FIRST_FILE_EX_A: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_FIRST_FILE_EX_W: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_NEXT_FILE_A: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_NEXT_FILE_W: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_CLOSE: AtomicUsize = AtomicUsize::new(0);

/// 合成枚举句柄 → 枚举状态
static SYNTHETIC: OnceLock<Mutex<HashMap<usize, SyntheticState>>> = OnceLock::new();
static NEXT_SYNTHETIC: AtomicUsize = AtomicUsize::new(0);

struct SyntheticState {
    /// 真实枚举结束后才吐出的虚拟条目
    injected: Vec<String>,
    /// 真实枚举句柄；根目录不存在时为 0（此时虚拟条目立即可用）
    real: usize,
    /// 真实枚举是否已结束
    real_done: bool,
    /// 该句柄属于宽字符还是 ANSI 变体（决定填充与转交哪个 API）
    wide: bool,
}

fn synthetic() -> &'static Mutex<HashMap<usize, SyntheticState>> {
    SYNTHETIC.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 安装枚举注入；须在 `platform` 阶段（`path_hook` 安装完成）之后调用
///
/// 游戏只导入其中一部分 API，缺失的跳过而不是整体失败。
pub fn install(api: &Api) -> Result<(), String> {
    let mut bound = 0usize;
    let mut missing: Vec<&str> = Vec::new();

    for (name, detour, slot) in hooks() {
        // 只挂游戏主模块：path_hook 已把它的下一跳铺进各模块的 IAT
        let previous = unsafe { hook_iat(api.game_base(), KERNEL32_DLL, name, detour) };
        match previous {
            Some(previous) => {
                slot.store(previous as usize, Ordering::SeqCst);
                bound += 1;
            }
            None => missing.push(name),
        }
    }

    if bound == 0 {
        return Err("no directory enumeration import found in game module".to_owned());
    }
    api.log_info(&format!(
        "ServerResources: option enumeration hook installed ({bound} APIs)"
    ));
    if !missing.is_empty() {
        api.log_warn(&format!(
            "ServerResources: not imported by game module, skipped: {}",
            missing.join(", ")
        ));
    }
    Ok(())
}

/// 需要挂接的 API 表
fn hooks() -> [(&'static str, *const (), &'static AtomicUsize); 7] {
    [
        (
            "FindFirstFileA",
            hooked_find_first_file_a as *const (),
            &NEXT_FIND_FIRST_FILE_A,
        ),
        (
            "FindFirstFileW",
            hooked_find_first_file_w as *const (),
            &NEXT_FIND_FIRST_FILE_W,
        ),
        (
            "FindFirstFileExA",
            hooked_find_first_file_ex_a as *const (),
            &NEXT_FIND_FIRST_FILE_EX_A,
        ),
        (
            "FindFirstFileExW",
            hooked_find_first_file_ex_w as *const (),
            &NEXT_FIND_FIRST_FILE_EX_W,
        ),
        (
            "FindNextFileA",
            hooked_find_next_file_a as *const (),
            &NEXT_FIND_NEXT_FILE_A,
        ),
        (
            "FindNextFileW",
            hooked_find_next_file_w as *const (),
            &NEXT_FIND_NEXT_FILE_W,
        ),
        (
            "FindClose",
            hooked_find_close as *const (),
            &NEXT_FIND_CLOSE,
        ),
    ]
}

/// 卸载：还原导入（把下一跳写回）并丢弃未完成的枚举状态
pub fn shutdown() {
    if let Ok(mut synthetic) = synthetic().lock() {
        synthetic.clear();
    }
    for (name, _, slot) in hooks() {
        unsafe { restore(name, slot) };
    }
}

unsafe fn restore(name: &str, slot: &AtomicUsize) {
    let previous = slot.swap(0, Ordering::AcqRel);
    if previous == 0 {
        return;
    }
    if let Some(api) = crate::util::api::API.get() {
        let _ = hook_iat(api.game_base(), KERNEL32_DLL, name, previous as *const ());
    }
}

/// 枚举通配路径是否正对 option 根（`c:\mount\option\*`）
///
/// 游戏请求的是重定向前的源路径。通配符必须落在最后一段，且目录必须**正好**
/// 是 option 根——更深一层已由 `platform/vfs` 重定向到真实目录，游戏自己能枚举。
fn is_option_root_glob(path: &str) -> bool {
    let normalized = winapi::normalize_path(path);
    let Some(star) = normalized.rfind('*') else {
        return false;
    };
    if normalized[star..].contains('\\') {
        return false;
    }
    normalized[..star].trim_end_matches('\\') == option_source()
}

fn option_source() -> String {
    vfs::option_source_prefix()
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

// ---------------------------------------------------------------------------
// `WIN32_FIND_DATA` 读写（A 与 W 共用偏移）
// ---------------------------------------------------------------------------

unsafe fn fill_find_data(name: &str, data: *mut c_void, wide: bool) {
    if data.is_null() {
        return;
    }
    let size = if wide {
        FIND_DATA_SIZE_W
    } else {
        FIND_DATA_SIZE_A
    };
    std::ptr::write_bytes(data as *mut u8, 0, size);
    // 注入的都是 option 根目录
    std::ptr::write_unaligned(data as *mut u32, FILE_ATTRIBUTE_DIRECTORY);

    let target = (data as *mut u8).add(FIND_DATA_NAME_OFFSET);
    if wide {
        let units = target.cast::<u16>();
        for (index, unit) in name.encode_utf16().take(FIND_DATA_NAME_MAX - 1).enumerate() {
            std::ptr::write_unaligned(units.add(index), unit);
        }
    } else {
        for (index, byte) in name.bytes().take(FIND_DATA_NAME_MAX - 1).enumerate() {
            *target.add(index) = byte;
        }
    }
}

unsafe fn name_from(data: *const c_void, wide: bool) -> String {
    if data.is_null() {
        return String::new();
    }
    let name = (data as *const u8).add(FIND_DATA_NAME_OFFSET);
    let mut length = 0usize;
    if wide {
        let units = name.cast::<u16>();
        while length < FIND_DATA_NAME_MAX && *units.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(units, length))
    } else {
        while length < FIND_DATA_NAME_MAX && *name.add(length) != 0 {
            length += 1;
        }
        String::from_utf8_lossy(std::slice::from_raw_parts(name, length)).into_owned()
    }
}

// ---------------------------------------------------------------------------
// 枚举注入
// ---------------------------------------------------------------------------

fn next_synthetic_handle() -> usize {
    let token = NEXT_SYNTHETIC.fetch_add(1, Ordering::Relaxed);
    token | SYNTHETIC_FLAG
}

/// 把已挂载的自定义 option 根接到 `real` 枚举结果之后
///
/// `real` 是调用方已经取到的真实枚举结果（也可能失败）。真实根目录存在时真实
/// 条目先行、虚拟条目排在其后；不存在时用合成句柄直接交付虚拟条目。
unsafe fn inject_option_roots(real: HANDLE, data: *mut c_void, wide: bool) -> HANDLE {
    let roots = crate::server_resources::mounted_roots();
    if roots.is_empty() {
        return real;
    }

    if real == INVALID_HANDLE_VALUE {
        // 真实 option 目录不存在：合成句柄，首个条目随 FindFirstFile 一起交付
        let mut injected: Vec<String> = roots.into_iter().rev().collect();
        let Some(first) = injected.pop() else {
            return real;
        };
        let Some(handle_value) = reserve_synthetic(SyntheticState {
            injected,
            real: 0,
            real_done: true,
            wide,
        }) else {
            return real;
        };
        fill_find_data(&first, data, wide);
        return handle_value as HANDLE;
    }

    // 真实根目录存在：真实条目先行，虚拟根排在最后；同名则不再重复
    let first = name_from(data, wide);
    let injected: Vec<String> = roots
        .into_iter()
        .filter(|root| !root.eq_ignore_ascii_case(&first))
        .rev()
        .collect();
    if injected.is_empty() {
        return real;
    }
    let Some(handle_value) = reserve_synthetic(SyntheticState {
        injected,
        real: real as usize,
        real_done: false,
        wide,
    }) else {
        return real;
    };
    handle_value as HANDLE
}

/// 登记一个合成枚举状态；超出上限时返回 `None`（调用方回退真实结果）
fn reserve_synthetic(state: SyntheticState) -> Option<usize> {
    let mut synthetic = synthetic().lock().ok()?;
    if synthetic.len() >= MAX_SYNTHETIC {
        log_warn("ServerResources: too many open enumerations, skipping injection");
        return None;
    }
    let handle_value = next_synthetic_handle();
    synthetic.insert(handle_value, state);
    Some(handle_value)
}

/// 合成句柄的下一项；不是合成句柄时返回 `None`（调用方转交真实 API）
unsafe fn synthetic_next(handle: HANDLE, data: *mut c_void) -> Option<i32> {
    if (handle as usize) & SYNTHETIC_FLAG == 0 {
        return None;
    }
    let Ok(mut synthetic) = synthetic().lock() else {
        return Some(0);
    };
    let Some(state) = synthetic.get_mut(&(handle as usize)) else {
        return Some(0);
    };

    // 先走真实枚举；结束后再追加虚拟条目，保证原有顺序不变
    if !state.real_done {
        if state.real != 0 {
            let slot = if state.wide {
                &NEXT_FIND_NEXT_FILE_W
            } else {
                &NEXT_FIND_NEXT_FILE_A
            };
            if let Some(next) = next_fn::<FindNextFileFn>(slot) {
                if next(state.real as HANDLE, data) != 0 {
                    // 真实目录已有同名项则不再追加虚拟根，避免重复
                    let name = name_from(data, state.wide);
                    state
                        .injected
                        .retain(|root| !root.eq_ignore_ascii_case(&name));
                    return Some(1);
                }
            }
        }
        state.real_done = true;
    }

    match state.injected.pop() {
        Some(root) => {
            fill_find_data(&root, data, state.wide);
            Some(1)
        }
        None => Some(0),
    }
}

/// 关闭合成句柄，回收其真实句柄
unsafe fn synthetic_close(handle: HANDLE) -> Option<i32> {
    if (handle as usize) & SYNTHETIC_FLAG == 0 {
        return None;
    }
    let real = synthetic()
        .lock()
        .ok()
        .and_then(|mut synthetic| synthetic.remove(&(handle as usize)).map(|state| state.real))
        .unwrap_or(0);
    if real != 0 {
        if let Some(next) = next_fn::<FindCloseFn>(&NEXT_FIND_CLOSE) {
            next(real as HANDLE);
        }
    }
    Some(1)
}

unsafe fn inject_from_path(
    path: Option<String>,
    real: HANDLE,
    data: *mut c_void,
    wide: bool,
) -> HANDLE {
    match path {
        Some(path) if is_option_root_glob(&path) => inject_option_roots(real, data, wide),
        _ => real,
    }
}

// ---------------------------------------------------------------------------
// 枚举钩子
// ---------------------------------------------------------------------------

unsafe extern "system" fn hooked_find_first_file_a(path: *const i8, data: *mut c_void) -> HANDLE {
    let Some(next) = next_fn::<FindFirstFileAFn>(&NEXT_FIND_FIRST_FILE_A) else {
        return INVALID_HANDLE_VALUE;
    };
    let real = next(path, data);
    let text = winapi::cstr_to_string(path);
    inject_from_path(text, real, data, false)
}

unsafe extern "system" fn hooked_find_first_file_w(path: *const u16, data: *mut c_void) -> HANDLE {
    let Some(next) = next_fn::<FindFirstFileWFn>(&NEXT_FIND_FIRST_FILE_W) else {
        return INVALID_HANDLE_VALUE;
    };
    let real = next(path, data);
    let text = winapi::wide_to_string(path);
    inject_from_path(text, real, data, true)
}

unsafe extern "system" fn hooked_find_first_file_ex_a(
    path: *const i8,
    info_level: u32,
    data: *mut c_void,
    search_op: u32,
    filter: *mut c_void,
    flags: u32,
) -> HANDLE {
    let Some(next) = next_fn::<FindFirstFileExAFn>(&NEXT_FIND_FIRST_FILE_EX_A) else {
        return INVALID_HANDLE_VALUE;
    };
    let real = next(path, info_level, data, search_op, filter, flags);
    let text = winapi::cstr_to_string(path);
    inject_from_path(text, real, data, false)
}

unsafe extern "system" fn hooked_find_first_file_ex_w(
    path: *const u16,
    info_level: u32,
    data: *mut c_void,
    search_op: u32,
    filter: *mut c_void,
    flags: u32,
) -> HANDLE {
    let Some(next) = next_fn::<FindFirstFileExWFn>(&NEXT_FIND_FIRST_FILE_EX_W) else {
        return INVALID_HANDLE_VALUE;
    };
    let real = next(path, info_level, data, search_op, filter, flags);
    let text = winapi::wide_to_string(path);
    inject_from_path(text, real, data, true)
}

unsafe extern "system" fn hooked_find_next_file_a(handle: HANDLE, data: *mut c_void) -> i32 {
    if let Some(result) = synthetic_next(handle, data) {
        return result;
    }
    match next_fn::<FindNextFileFn>(&NEXT_FIND_NEXT_FILE_A) {
        Some(next) => next(handle, data),
        None => 0,
    }
}

unsafe extern "system" fn hooked_find_next_file_w(handle: HANDLE, data: *mut c_void) -> i32 {
    if let Some(result) = synthetic_next(handle, data) {
        return result;
    }
    match next_fn::<FindNextFileFn>(&NEXT_FIND_NEXT_FILE_W) {
        Some(next) => next(handle, data),
        None => 0,
    }
}

unsafe extern "system" fn hooked_find_close(handle: HANDLE) -> i32 {
    if let Some(result) = synthetic_close(handle) {
        return result;
    }
    match next_fn::<FindCloseFn>(&NEXT_FIND_CLOSE) {
        Some(next) => next(handle),
        None => 0,
    }
}

/// 取出下一跳函数指针；未挂接时为 `None`
unsafe fn next_fn<T>(slot: &AtomicUsize) -> Option<T> {
    let value = slot.load(Ordering::SeqCst);
    if value == 0 {
        return None;
    }
    Some(std::mem::transmute_copy::<usize, T>(&value))
}

fn log_warn(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_warn(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_handles_are_flagged_and_unique() {
        // Given: 连续分配两个合成枚举句柄。
        let first = next_synthetic_handle();
        let second = next_synthetic_handle();

        // Then: 都带高位标记且互不相同，不会与内核句柄混淆。
        assert_ne!(first, second);
        assert_ne!(first & SYNTHETIC_FLAG, 0);
        assert_ne!(second & SYNTHETIC_FLAG, 0);
    }

    #[test]
    fn only_the_option_root_glob_is_claimed() {
        // Given: option 根的各种通配写法。
        // Then: 只有根那一级被认领；更深一层由重定向交给游戏自己枚举。
        assert!(is_option_root_glob("c:\\mount\\option\\*"));
        assert!(is_option_root_glob("C:\\mount\\option\\*.*"));
        assert!(!is_option_root_glob("c:\\mount\\option\\A123\\*"));
        assert!(!is_option_root_glob("c:\\mount\\option2\\*"));
        assert!(!is_option_root_glob("c:\\mount\\option\\*\\data.conf"));
        assert!(!is_option_root_glob("c:\\mount\\option\\A123\\data.conf"));
    }

    #[test]
    fn find_data_name_round_trips_in_both_variants() {
        // Given: 分别按 ANSI / 宽字符填充的 FIND_DATA 缓冲区。
        let mut ansi = vec![0u8; FIND_DATA_SIZE_A];
        let mut wide = vec![0u8; FIND_DATA_SIZE_W];

        unsafe {
            // When: 写入 option 根目录条目。
            fill_find_data("A123", ansi.as_mut_ptr().cast(), false);
            fill_find_data("A124", wide.as_mut_ptr().cast(), true);

            // Then: 名称与目录属性都能读回。
            assert_eq!(name_from(ansi.as_ptr().cast(), false), "A123");
            assert_eq!(name_from(wide.as_ptr().cast(), true), "A124");
            assert_eq!(
                u32::from_le_bytes(ansi[0..4].try_into().unwrap()),
                FILE_ATTRIBUTE_DIRECTORY
            );
            assert_eq!(
                u32::from_le_bytes(wide[0..4].try_into().unwrap()),
                FILE_ATTRIBUTE_DIRECTORY
            );
        }
    }

    #[test]
    fn state_keeps_real_handle_after_injection() {
        // Given: 一次带真实句柄的枚举。
        let handle_value = next_synthetic_handle();
        synthetic().lock().unwrap().insert(
            handle_value,
            SyntheticState {
                injected: Vec::new(),
                real: 0x1234,
                real_done: false,
                wide: false,
            },
        );

        // Then: 关闭时取出真实句柄（调用方据此转交 FindClose），状态被清除。
        unsafe {
            assert_eq!(synthetic_close(handle_value as HANDLE), Some(1));
        }
        assert!(!synthetic().lock().unwrap().contains_key(&handle_value));
    }

    #[test]
    fn virtual_roots_are_deferred_until_real_enumeration_ends() {
        // Given: 一个真实枚举已结束的合成句柄，带两个虚拟根（倒序存放）。
        let handle_value = next_synthetic_handle();
        synthetic().lock().unwrap().insert(
            handle_value,
            SyntheticState {
                injected: vec!["A124".to_owned(), "A123".to_owned()],
                real: 0,
                real_done: false,
                wide: false,
            },
        );

        // When: 真实句柄为 0（根目录不存在），逐个取虚拟条目。
        let mut buffer = vec![0u8; FIND_DATA_SIZE_A];
        let mut names = Vec::new();
        unsafe {
            while synthetic_next(handle_value as HANDLE, buffer.as_mut_ptr().cast()) == Some(1) {
                names.push(name_from(buffer.as_ptr().cast(), false));
            }
        }

        // Then: 按挂载顺序交出，取完后返回 0。
        assert_eq!(names, vec!["A123".to_owned(), "A124".to_owned()]);
    }

    #[test]
    fn unknown_handles_are_left_to_real_api() {
        // Given: 不带合成标记的内核句柄。
        // Then: 枚举与关闭都不认领。
        unsafe {
            assert_eq!(synthetic_next(0x1234 as HANDLE, std::ptr::null_mut()), None);
            assert_eq!(synthetic_close(0x1234 as HANDLE), None);
        }
    }
}
