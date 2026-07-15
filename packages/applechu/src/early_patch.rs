use std::ffi::c_void;
use std::sync::Mutex;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::config::Config;
use crate::patches;
use crate::util::api::Api;
use crate::util::memory::PatchMemory;

enum EarlyLog {
    Info(String),
    Warn(String),
}

static EARLY_LOGS: Mutex<Vec<EarlyLog>> = Mutex::new(Vec::new());

struct DirectMemory {
    game_base: usize,
    game_size: u32,
}

pub unsafe fn apply(game_base: usize) {
    if game_base == 0 {
        return;
    }
    let game_size = crate::proxy::game_size(game_base);
    let Some(base_dir) = crate::proxy::base_dir() else {
        return;
    };
    if game_size == 0 {
        return;
    }

    let memory = DirectMemory {
        game_base,
        game_size,
    };
    let config = Config::global(&base_dir);
    apply_pre_tls_if_valid(&memory, config);
}

fn apply_pre_tls_if_valid<M: PatchMemory>(memory: &M, config: &Config) {
    if !config.is_valid() {
        memory.log_warn("early patch skipped: invalid AppleChu.toml");
        return;
    }
    patches::apply_pre_tls(memory, config);
}

pub fn flush_logs(api: &Api) {
    let logs = match EARLY_LOGS.lock() {
        Ok(mut logs) => std::mem::take(&mut *logs),
        Err(_) => return,
    };
    for log in logs {
        match log {
            EarlyLog::Info(message) => api.log_info(&message),
            EarlyLog::Warn(message) => api.log_warn(&message),
        }
    }
}

impl DirectMemory {
    fn contains_range(&self, address: usize, len: usize) -> bool {
        let Some(offset) = address.checked_sub(self.game_base) else {
            return false;
        };
        offset
            .checked_add(len)
            .is_some_and(|end| end <= self.game_size as usize)
    }
}

impl PatchMemory for DirectMemory {
    fn game_base(&self) -> usize {
        self.game_base
    }

    fn game_size(&self) -> u32 {
        self.game_size
    }

    fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize {
        if !self.contains_range(start, size as usize) {
            return 0;
        }
        // SAFETY: Category 3/10（悬空与越界）。范围已限制在 Windows 已映射的游戏 PE 映像内，
        // 进程卸载主模块前该映像始终有效，且扫描期间不写入。
        let image = unsafe { std::slice::from_raw_parts(start as *const u8, size as usize) };
        find_pattern(image, pattern, mask).map_or(0, |offset| start + offset)
    }

    fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool {
        if !self.contains_range(addr, buf.len()) {
            return false;
        }
        // SAFETY: Category 3/10（悬空与越界）。目标范围已验证属于仍映射的主模块，
        // 目标切片与调用方缓冲区互不重叠。
        unsafe {
            std::ptr::copy_nonoverlapping(addr as *const u8, buf.as_mut_ptr(), buf.len());
        }
        true
    }

    fn mem_write(&self, addr: usize, data: &[u8]) -> bool {
        if data.is_empty() || !self.contains_range(addr, data.len()) {
            return false;
        }

        let address = addr as *mut u8;
        let mut old_protect = 0;
        // SAFETY: Category 8/10（FFI 与越界）。地址范围已验证属于主模块；VirtualProtect
        // 只临时放宽该已提交映像页，写入长度与校验过的 data 完全一致。
        if unsafe {
            VirtualProtect(
                address.cast(),
                data.len(),
                PAGE_EXECUTE_READWRITE,
                &mut old_protect,
            )
        } == 0
        {
            return false;
        }

        // SAFETY: Category 1/10（别名与越界）。补丁源位于 Rust slice，目标位于游戏映像，
        // 两者不重叠；目标长度已由 contains_range 证明。
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), address, data.len());
        }

        let mut ignored = 0;
        // SAFETY: Category 8（FFI）。参数复用成功 VirtualProtect 返回的页保护值与同一范围。
        let _ = unsafe { VirtualProtect(address.cast(), data.len(), old_protect, &mut ignored) };
        // SAFETY: Category 8（FFI）。写入范围仍属于当前进程映像，刷新后 CPU 才能看到新指令。
        let _ = unsafe {
            FlushInstructionCache(GetCurrentProcess(), address.cast::<c_void>(), data.len())
        };
        true
    }

    fn log_info(&self, message: &str) {
        if let Ok(mut logs) = EARLY_LOGS.lock() {
            logs.push(EarlyLog::Info(message.to_owned()));
        }
    }

    fn log_warn(&self, message: &str) {
        if let Ok(mut logs) = EARLY_LOGS.lock() {
            logs.push(EarlyLog::Warn(message.to_owned()));
        }
    }
}

fn find_pattern(image: &[u8], pattern: &[u8], mask: &str) -> Option<usize> {
    if pattern.is_empty() || pattern.len() != mask.len() || pattern.len() > image.len() {
        return None;
    }
    let mask = mask.as_bytes();
    image.windows(pattern.len()).position(|window| {
        window
            .iter()
            .zip(pattern)
            .zip(mask)
            .all(|((&actual, &expected), &kind)| kind == b'?' || actual == expected)
    })
}

#[cfg(test)]
mod tests;
