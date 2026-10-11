//! Option VFS 桥接
//!
//! 把内存中的 Option 包呈现给游戏：枚举目录时注入虚拟 `AXXX` 根，
//! 读取文件时由内存内容服务。资源不落盘，磁盘上只保留缓存包本身。
//!
//! ## 挂在哪一层
//!
//! `platform/path_hook` 已经把游戏的 option 访问改写到一个可变根目录
//! （默认 `<游戏目录>\option`），但只做路径改写、不注入条目。这里在**其之上**
//! 再包一层：保留 `path_hook` 的原始指针，自己优先处理虚拟内容，未命中时
//! 转交 `path_hook`（按路径改写）再落到系统 API。
//!
//! ## 关键点
//!
//! - 目录枚举用「合成句柄」：只在 `path_hook` 的原始 `FindFirstFileW` 失败
//!   （目标目录不存在）时才创建，避免与真实句柄混淆；合成句柄高位标记，
//!   不会与内核句柄冲突。
//! - 文件内容用**真实的 `\\.\NUL` 句柄**承载：`CreateFileW` 命中虚拟路径时
//!   打开 NUL，并把真实句柄登记进 `handle` 表；`ReadFile` / `GetFileSize` /
//!   `SetFilePointer` / `CloseHandle` 按句柄查表即可区分虚拟与真实文件。
//!   即便某个真实 API 没被钩到，拿到 NUL 句柄也只是读到 0 字节，不会误读磁盘。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

// 这些导入在 Windows 上是真实插件依赖；合成句柄用高位掩码避开内核句柄空间。
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, WIN32_FIND_DATAW,
};

use crate::platform::{vfs, winapi};
use crate::server_resources::handle;
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;

const KERNEL32_DLL: &str = "kernel32.dll";
/// `INVALID_FILE_ATTRIBUTES`
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
/// `GetFileSize` 失败返回值
const INVALID_FILE_SIZE: u32 = u32::MAX;
/// 合成枚举句柄的高位标记（内核句柄不会落到该区间）
const SYNTHETIC_FLAG: usize = 1 << 31;
/// `SetFilePointer` 的 `FILE_BEGIN`
const FILE_BEGIN: u32 = 0;

type FindFirstFileWFn = unsafe extern "system" fn(*const u16, *mut WIN32_FIND_DATAW) -> HANDLE;
type FindNextFileWFn = unsafe extern "system" fn(HANDLE, *mut WIN32_FIND_DATAW) -> i32;
type FindCloseFn = unsafe extern "system" fn(HANDLE) -> i32;
type CreateFileWFn =
    unsafe extern "system" fn(*const u16, u32, u32, *const c_void, u32, u32, HANDLE) -> HANDLE;
type ReadFileFn = unsafe extern "system" fn(HANDLE, *mut c_void, u32, *mut u32, *mut c_void) -> i32;
type GetFileSizeFn = unsafe extern "system" fn(HANDLE, *mut u32) -> u32;
type GetFileSizeExFn = unsafe extern "system" fn(HANDLE, *mut i64) -> i32;
type SetFilePointerFn = unsafe extern "system" fn(HANDLE, i32, *mut i32, u32) -> u32;
type SetFilePointerExFn = unsafe extern "system" fn(HANDLE, i64, *mut i64, u32) -> i32;
type CloseHandleFn = unsafe extern "system" fn(HANDLE) -> i32;

/// 我们包装后的 API 指针（未命中虚拟内容时转交）
static NEXT_FIND_FIRST_FILE_W: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_NEXT_FILE_W: AtomicUsize = AtomicUsize::new(0);
static NEXT_FIND_CLOSE: AtomicUsize = AtomicUsize::new(0);
static NEXT_CREATE_FILE_W: AtomicUsize = AtomicUsize::new(0);
static NEXT_READ_FILE: AtomicUsize = AtomicUsize::new(0);
static NEXT_GET_FILE_SIZE: AtomicUsize = AtomicUsize::new(0);
static NEXT_GET_FILE_SIZE_EX: AtomicUsize = AtomicUsize::new(0);
static NEXT_SET_FILE_POINTER: AtomicUsize = AtomicUsize::new(0);
static NEXT_SET_FILE_POINTER_EX: AtomicUsize = AtomicUsize::new(0);
static NEXT_CLOSE_HANDLE: AtomicUsize = AtomicUsize::new(0);

/// 合成枚举句柄 → 待吐出的条目（倒序弹出）
static SYNTHETIC: OnceLock<Mutex<HashMap<usize, Vec<InjectedEntry>>>> = OnceLock::new();
static NEXT_SYNTHETIC: AtomicUsize = AtomicUsize::new(0);

struct InjectedEntry {
    name: String,
    directory: bool,
}

fn synthetic() -> &'static Mutex<HashMap<usize, Vec<InjectedEntry>>> {
    SYNTHETIC.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 安装 VFS 桥接；须在 `platform` 阶段（`path_hook` 安装完成）之后调用
pub fn install(api: &Api) -> Result<(), String> {
    unsafe {
        bind(
            api,
            "FindFirstFileW",
            hooked_find_first_file_w as *const (),
            &NEXT_FIND_FIRST_FILE_W,
        )?;
        bind(
            api,
            "FindNextFileW",
            hooked_find_next_file_w as *const (),
            &NEXT_FIND_NEXT_FILE_W,
        )?;
        bind(
            api,
            "FindClose",
            hooked_find_close as *const (),
            &NEXT_FIND_CLOSE,
        )?;
        bind(
            api,
            "CreateFileW",
            hooked_create_file_w as *const (),
            &NEXT_CREATE_FILE_W,
        )?;
        bind(
            api,
            "ReadFile",
            hooked_read_file as *const (),
            &NEXT_READ_FILE,
        )?;
        bind(
            api,
            "GetFileSize",
            hooked_get_file_size as *const (),
            &NEXT_GET_FILE_SIZE,
        )?;
        bind(
            api,
            "GetFileSizeEx",
            hooked_get_file_size_ex as *const (),
            &NEXT_GET_FILE_SIZE_EX,
        )?;
        bind(
            api,
            "SetFilePointer",
            hooked_set_file_pointer as *const (),
            &NEXT_SET_FILE_POINTER,
        )?;
        bind(
            api,
            "SetFilePointerEx",
            hooked_set_file_pointer_ex as *const (),
            &NEXT_SET_FILE_POINTER_EX,
        )?;
        bind(
            api,
            "CloseHandle",
            hooked_close_handle as *const (),
            &NEXT_CLOSE_HANDLE,
        )?;
    }
    api.log_info("ServerResources: option VFS bridge installed");
    Ok(())
}

/// 把 detour 挂到目标进程内该导入的**当前**实现上，并保留它作为下一跳
unsafe fn bind(api: &Api, name: &str, detour: *const (), slot: &AtomicUsize) -> Result<(), String> {
    // 只挂游戏主模块：避免与 path_hook 覆盖的其他模块重复包装
    let previous = hook_iat(api.game_base(), KERNEL32_DLL, name, detour);
    let Some(previous) = previous else {
        return Err(format!("{name} import not found in game module"));
    };
    slot.store(previous as usize, Ordering::SeqCst);
    Ok(())
}

/// 卸载：还原导入（把下一跳写回）
pub fn shutdown() {
    handle::close_all();
    if let Ok(mut synthetic) = synthetic().lock() {
        synthetic.clear();
    }
    unsafe {
        restore("FindFirstFileW", &NEXT_FIND_FIRST_FILE_W);
        restore("FindNextFileW", &NEXT_FIND_NEXT_FILE_W);
        restore("FindClose", &NEXT_FIND_CLOSE);
        restore("CreateFileW", &NEXT_CREATE_FILE_W);
        restore("ReadFile", &NEXT_READ_FILE);
        restore("GetFileSize", &NEXT_GET_FILE_SIZE);
        restore("GetFileSizeEx", &NEXT_GET_FILE_SIZE_EX);
        restore("SetFilePointer", &NEXT_SET_FILE_POINTER);
        restore("SetFilePointerEx", &NEXT_SET_FILE_POINTER_EX);
        restore("CloseHandle", &NEXT_CLOSE_HANDLE);
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

/// 命中 option 根通配时返回包内根目录名
///
/// 游戏请求的是重定向前的源路径 `c:\mount\option\*`，因此用源前缀比对。
fn option_root_glob(path: &str) -> Option<String> {
    let source = vfs::option_source_prefix();
    let normalized = winapi::normalize_path(path);
    let directory = normalized.trim_end_matches('\\').trim_end_matches("*");
    if directory.trim_end_matches('\\') != source.trim_end_matches('\\') {
        return None;
    }
    crate::server_resources::package().map(|package| package.root().to_owned())
}

/// 路径是否指向虚拟包内文件，返回包内 key
fn package_key(path: &str) -> Option<String> {
    let package = crate::server_resources::package()?;
    let source = vfs::option_source_prefix();
    let normalized = winapi::normalize_path(path);
    let prefix = format!("{}\\", source.trim_end_matches('\\'));
    let relative = normalized.strip_prefix(&prefix)?;
    let key = relative.replace('\\', "/");
    package.contains(&key).then_some(key)
}

/// 把名称写入 `WIN32_FIND_DATAW`
unsafe fn fill_entry(name: &str, directory: bool, data: *mut WIN32_FIND_DATAW) {
    if data.is_null() {
        return;
    }
    std::ptr::write_bytes(data, 0, 1);
    (*data).dwFileAttributes = if directory {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_NORMAL
    };
    let wide = winapi::string_to_wide(name);
    let units = wide.len().min(259);
    std::ptr::copy_nonoverlapping(wide.as_ptr(), (*data).cFileName.as_mut_ptr(), units);
}

fn next_synthetic_handle() -> usize {
    let token = NEXT_SYNTHETIC.fetch_add(1, Ordering::Relaxed);
    token | SYNTHETIC_FLAG
}

unsafe extern "system" fn hooked_find_first_file_w(
    path: *const u16,
    data: *mut WIN32_FIND_DATAW,
) -> HANDLE {
    let next = NEXT_FIND_FIRST_FILE_W.load(Ordering::SeqCst);
    if next == 0 {
        return INVALID_HANDLE_VALUE;
    }
    let next: FindFirstFileWFn = std::mem::transmute(next);

    if let Some(path_text) = winapi::wide_to_string(path) {
        if let Some(root) = option_root_glob(&path_text) {
            // 先让真实枚举跑一遍：目录存在时交给它（本功能只做增量挂载）
            let real = next(path, data);
            if real != INVALID_HANDLE_VALUE {
                return real;
            }
            // 目录不存在：用合成句柄承载注入条目
            let handle_value = next_synthetic_handle();
            if let Ok(mut synthetic) = synthetic().lock() {
                synthetic.insert(handle_value, Vec::new());
            }
            fill_entry(&root, true, data);
            return handle_value as HANDLE;
        }
    }
    next(path, data)
}

unsafe extern "system" fn hooked_find_next_file_w(
    handle: HANDLE,
    data: *mut WIN32_FIND_DATAW,
) -> i32 {
    if (handle as usize) & SYNTHETIC_FLAG != 0 {
        let Ok(mut synthetic) = synthetic().lock() else {
            return 0;
        };
        if let Some(entries) = synthetic.get_mut(&(handle as usize)) {
            if let Some(entry) = entries.pop() {
                fill_entry(&entry.name, entry.directory, data);
                return 1;
            }
        }
        return 0;
    }
    let next = NEXT_FIND_NEXT_FILE_W.load(Ordering::SeqCst);
    if next == 0 {
        return 0;
    }
    let next: FindNextFileWFn = std::mem::transmute(next);
    next(handle, data)
}

unsafe extern "system" fn hooked_find_close(handle: HANDLE) -> i32 {
    if (handle as usize) & SYNTHETIC_FLAG != 0 {
        if let Ok(mut synthetic) = synthetic().lock() {
            synthetic.remove(&(handle as usize));
        }
        return 1;
    }
    let next = NEXT_FIND_CLOSE.load(Ordering::SeqCst);
    if next == 0 {
        return 0;
    }
    let next: FindCloseFn = std::mem::transmute(next);
    next(handle)
}

unsafe extern "system" fn hooked_create_file_w(
    path: *const u16,
    access: u32,
    share: u32,
    security: *const c_void,
    creation: u32,
    flags: u32,
    template: HANDLE,
) -> HANDLE {
    let next = NEXT_CREATE_FILE_W.load(Ordering::SeqCst);
    if next == 0 {
        return INVALID_HANDLE_VALUE;
    }
    let next: CreateFileWFn = std::mem::transmute(next);

    if let Some(path_text) = winapi::wide_to_string(path) {
        if let Some(key) = package_key(&path_text) {
            let length = crate::server_resources::package()
                .and_then(|package| package.read_file(&key).map(<[u8]>::len))
                .unwrap_or(0);
            // \\.\NUL 必然可打开，且 CloseHandle 语义正常
            let nul = winapi::string_to_wide("\\\\.\\NUL");
            let real = next(
                nul.as_ptr(),
                access,
                share,
                security,
                creation,
                flags,
                template,
            );
            if real != INVALID_HANDLE_VALUE && handle::register(real as usize, &key, length) {
                return real;
            }
        }
    }
    next(path, access, share, security, creation, flags, template)
}

unsafe extern "system" fn hooked_read_file(
    file: HANDLE,
    buffer: *mut c_void,
    to_read: u32,
    read: *mut u32,
    overlapped: *mut c_void,
) -> i32 {
    if let Some((key, offset, count)) = handle::read(file as usize, to_read as usize) {
        let mut transferred = 0u32;
        if let Some(package) = crate::server_resources::package() {
            if let Some(data) = package.read_file(&key) {
                let start = offset.min(data.len());
                let available = (data.len() - start).min(count);
                if available > 0 && !buffer.is_null() {
                    std::ptr::copy_nonoverlapping(
                        data[start..].as_ptr(),
                        buffer as *mut u8,
                        available,
                    );
                }
                transferred = available as u32;
            }
        }
        if !read.is_null() {
            *read = transferred;
        }
        return 1;
    }
    let next = NEXT_READ_FILE.load(Ordering::SeqCst);
    if next == 0 {
        return 0;
    }
    let next: ReadFileFn = std::mem::transmute(next);
    next(file, buffer, to_read, read, overlapped)
}

unsafe extern "system" fn hooked_get_file_size(file: HANDLE, high: *mut u32) -> u32 {
    if let Some((length, _)) = handle::size_and_position(file as usize) {
        if !high.is_null() {
            *high = ((length as u64) >> 32) as u32;
        }
        return (length as u64 & 0xFFFF_FFFF) as u32;
    }
    let next = NEXT_GET_FILE_SIZE.load(Ordering::SeqCst);
    if next == 0 {
        return INVALID_FILE_SIZE;
    }
    let next: GetFileSizeFn = std::mem::transmute(next);
    next(file, high)
}

unsafe extern "system" fn hooked_get_file_size_ex(file: HANDLE, size: *mut i64) -> i32 {
    if let Some((length, _)) = handle::size_and_position(file as usize) {
        if !size.is_null() {
            *size = length as i64;
        }
        return 1;
    }
    let next = NEXT_GET_FILE_SIZE_EX.load(Ordering::SeqCst);
    if next == 0 {
        return 0;
    }
    let next: GetFileSizeExFn = std::mem::transmute(next);
    next(file, size)
}

unsafe extern "system" fn hooked_set_file_pointer(
    file: HANDLE,
    distance: i32,
    high: *mut i32,
    method: u32,
) -> u32 {
    if let Some((_, position)) = handle::size_and_position(file as usize) {
        let base = if method == FILE_BEGIN {
            0isize
        } else {
            position as isize
        };
        let target = base.saturating_add(distance as isize).max(0) as usize;
        if handle::seek(file as usize, target).is_some() {
            if !high.is_null() {
                *high = ((target as u64) >> 32) as i32;
            }
            return (target as u64 & 0xFFFF_FFFF) as u32;
        }
    }
    let next = NEXT_SET_FILE_POINTER.load(Ordering::SeqCst);
    if next == 0 {
        return INVALID_FILE_SIZE;
    }
    let next: SetFilePointerFn = std::mem::transmute(next);
    next(file, distance, high, method)
}

unsafe extern "system" fn hooked_set_file_pointer_ex(
    file: HANDLE,
    distance: i64,
    new_position: *mut i64,
    method: u32,
) -> i32 {
    if let Some((_, position)) = handle::size_and_position(file as usize) {
        let base = if method == FILE_BEGIN {
            0
        } else {
            position as i64
        };
        let target = base.saturating_add(distance).max(0) as usize;
        if handle::seek(file as usize, target).is_some() {
            if !new_position.is_null() {
                *new_position = target as i64;
            }
            return 1;
        }
    }
    let next = NEXT_SET_FILE_POINTER_EX.load(Ordering::SeqCst);
    if next == 0 {
        return 0;
    }
    let next: SetFilePointerExFn = std::mem::transmute(next);
    next(file, distance, new_position, method)
}

unsafe extern "system" fn hooked_close_handle(object: HANDLE) -> i32 {
    handle::unregister(object as usize);
    let next = NEXT_CLOSE_HANDLE.load(Ordering::SeqCst);
    if next == 0 {
        return 0;
    }
    let next: CloseHandleFn = std::mem::transmute(next);
    next(object)
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
    fn no_option_root_means_no_injection() {
        // Given: VFS 尚未初始化。
        // Then: 所有查询都不命中，调用方完全回落真实 API。
        assert!(option_root_glob("C:\\x\\*").is_none());
        assert!(package_key("C:\\x\\A200\\data.conf").is_none());
    }
}
