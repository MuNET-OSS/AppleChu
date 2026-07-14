use std::ffi::c_void;

use windows_sys_loader::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys_loader::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys_loader::Win32::System::Threading::GetCurrentProcess;

use crate::config::Config;
use crate::patches;
use crate::util::memory::PatchMemory;

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
    let config = Config::load(&base_dir);
    patches::apply_early(&memory, &config);
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

    fn log_info(&self, _message: &str) {}

    fn log_warn(&self, _message: &str) {}
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
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct FakeMemory {
        base: usize,
        image: RefCell<Vec<u8>>,
    }

    impl PatchMemory for FakeMemory {
        fn game_base(&self) -> usize {
            self.base
        }

        fn game_size(&self) -> u32 {
            self.image.borrow().len() as u32
        }

        fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize {
            let offset = start - self.base;
            let image = self.image.borrow();
            find_pattern(&image[offset..offset + size as usize], pattern, mask)
                .map_or(0, |found| start + found)
        }

        fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool {
            let offset = addr - self.base;
            buf.copy_from_slice(&self.image.borrow()[offset..offset + buf.len()]);
            true
        }

        fn mem_write(&self, addr: usize, data: &[u8]) -> bool {
            let offset = addr - self.base;
            self.image.borrow_mut()[offset..offset + data.len()].copy_from_slice(data);
            true
        }

        fn log_info(&self, _message: &str) {}

        fn log_warn(&self, _message: &str) {}
    }

    #[test]
    fn early_scanner_matches_wildcards_before_tls() {
        // Given: 版本间变化的字节位于稳定指令签名中。
        let image = [0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x12, 0x80];
        let pattern = [0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x00, 0x80];

        // When: TLS 前扫描器使用与晚期 patch 相同的掩码。
        let found = find_pattern(&image, &pattern, "xxxxxx?x");

        // Then: 可变字节不会破坏版本无关识别。
        assert_eq!(found, Some(0));
    }

    #[test]
    fn enabled_checks_are_patched_by_shared_early_pipeline() {
        // Given: AppUser 与 TLS 检测仍是原字节，且配置明确启用两项绕过。
        let mut image = vec![0x90; 64];
        image[8..14].copy_from_slice(&[0x83, 0x7C, 0x24, 0x04, 0x00, 0x75]);
        image[24..40].copy_from_slice(&[
            0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x00, 0x80, 0x00, 0xEB, 0x02, 0x33, 0xF6, 0x8B,
            0x5B, 0x34,
        ]);
        let memory = FakeMemory {
            base: 0x1000,
            image: RefCell::new(image),
        };
        let config = Config {
            base_dir: ".".to_owned(),
            sections: "[BypassAppUser]\n[DisableTLS]"
                .parse()
                .expect("测试配置必须有效"),
        };

        // When: DLL_PROCESS_ATTACH 使用与晚期阶段共享的 patch 定义。
        patches::apply_early(&memory, &config);

        // Then: 两个会在 TLS 初始化中被缓存的检测都已提前改写。
        let image = memory.image.borrow();
        assert_eq!(image[13], 0xEB);
        assert_eq!(image[31], 0x00);
    }
}
