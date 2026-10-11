//! 虚拟文件句柄登记表
//!
//! 游戏的 option 读取走 `CreateFileW` → `ReadFile` / `GetFileSize` → `CloseHandle`。
//! 对命中内存 Option 包的路径，`CreateFileW` 钩子会先向系统要一个**真实句柄**
//! （打开 `\\.\NUL`，不落盘、必然成功），再把该真实句柄登记到本表。这样：
//!
//! - 句柄是内核分配的，天然唯一，不会与真实文件句柄冲突；
//! - `ReadFile` / `GetFileSize` / `SetFilePointer` / `CloseHandle` 钩子按句柄
//!   查表即可区分虚拟与真实文件；
//! - 即便某个真实 API 没被钩到，拿到 NUL 句柄也只是读到 0 字节，不会误读磁盘。
//!
//! 本模块只做登记与读取簿记，与目标平台无关，便于在主机上直接跑单元测试。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 同时打开的虚拟文件上限，避免异常调用把表撑爆
pub const MAX_OPEN: usize = 64;

struct OpenFile {
    /// 包内相对路径（如 `A200/data.conf`）
    key: String,
    /// 当前读取偏移
    position: usize,
    /// 文件长度
    length: usize,
}

static FILES: OnceLock<Mutex<HashMap<usize, OpenFile>>> = OnceLock::new();

fn files() -> &'static Mutex<HashMap<usize, OpenFile>> {
    FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 登记一个虚拟文件句柄；超出上限或重复登记返回 `false`
pub fn register(handle: usize, key: &str, length: usize) -> bool {
    let Ok(mut files) = files().lock() else {
        return false;
    };
    if handle == 0 || files.len() >= MAX_OPEN || files.contains_key(&handle) {
        return false;
    }
    files.insert(
        handle,
        OpenFile {
            key: key.to_owned(),
            position: 0,
            length,
        },
    );
    true
}

/// 注销句柄，返回是否命中
pub fn unregister(handle: usize) -> bool {
    files()
        .lock()
        .map(|mut files| files.remove(&handle).is_some())
        .unwrap_or(false)
}

pub fn is_virtual(handle: usize) -> bool {
    files()
        .lock()
        .map(|files| files.contains_key(&handle))
        .unwrap_or(false)
}

/// 读取簿记：返回 (包内路径, 文件内偏移, 本次可读字节数)
///
/// `requested` 为 0 时按「读到文件剩余部分」处理，与 `ReadFile` 一致。
pub fn read(handle: usize, requested: usize) -> Option<(String, usize, usize)> {
    let mut files = files().lock().ok()?;
    let file = files.get_mut(&handle)?;
    let remaining = file.length.saturating_sub(file.position);
    let count = if requested == 0 {
        remaining
    } else {
        requested.min(remaining)
    };
    let offset = file.position;
    file.position += count;
    Some((file.key.clone(), offset, count))
}

/// 当前 (文件长度, 读取偏移)
pub fn size_and_position(handle: usize) -> Option<(usize, usize)> {
    let files = files().lock().ok()?;
    files.get(&handle).map(|file| (file.length, file.position))
}

/// 设置读取偏移；允许越界，与 `SetFilePointer` 语义一致
pub fn seek(handle: usize, position: usize) -> Option<usize> {
    let mut files = files().lock().ok()?;
    let file = files.get_mut(&handle)?;
    file.position = position;
    Some(file.position)
}

/// 清空全部登记（模块卸载时调用）
pub fn close_all() {
    if let Ok(mut files) = files().lock() {
        files.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_and_reads_sequentially() {
        // Given: 登记一个 10 字节的虚拟文件。
        assert!(register(0x1000, "A200/data.conf", 10));

        // When: 连续读取两次。
        let first = read(0x1000, 4).unwrap();
        let second = read(0x1000, 100).unwrap();

        // Then: 偏移递增，第二次被限制在剩余 6 字节内。
        assert_eq!(first, ("A200/data.conf".to_owned(), 0, 4));
        assert_eq!(second, ("A200/data.conf".to_owned(), 4, 6));
        assert_eq!(size_and_position(0x1000), Some((10, 10)));
        assert!(unregister(0x1000));
    }

    #[test]
    fn zero_length_request_reads_to_end() {
        // Given: 一个 5 字节文件。
        register(0x2000, "A200/x", 5);

        // When: 请求长度为 0（ReadFile 的常见写法）。
        let chunk = read(0x2000, 0).unwrap();

        // Then: 返回整个文件。
        assert_eq!(chunk.2, 5);
        assert!(unregister(0x2000));
    }

    #[test]
    fn seek_repositions_and_allows_past_end() {
        // Given: 一个 8 字节文件。
        register(0x3000, "A200/y", 8);

        // When: 定位到 20（越过末尾）再读取。
        assert_eq!(seek(0x3000, 20), Some(20));
        let chunk = read(0x3000, 4).unwrap();

        // Then: 越界读取返回 0 字节而不是回绕。
        assert_eq!(chunk.2, 0);
        assert!(unregister(0x3000));
    }

    #[test]
    fn unknown_handle_is_not_virtual() {
        // Given: 未登记的句柄。
        // Then: 查询与读取都不命中，调用方回落到真实 API。
        assert!(!is_virtual(0xDEAD));
        assert!(read(0xDEAD, 1).is_none());
        assert!(!unregister(0xDEAD));
    }

    #[test]
    fn duplicate_and_zero_handles_are_rejected() {
        // Given: 合法登记一次。
        assert!(register(0x4000, "A200/z", 1));

        // Then: 重复登记与 0 句柄都被拒绝。
        assert!(!register(0x4000, "A200/other", 1));
        assert!(!register(0, "A200/zero", 1));
        assert!(unregister(0x4000));
    }
}
