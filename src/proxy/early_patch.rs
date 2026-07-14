use std::ffi::c_void;
use std::path::Path;

use windows_sys_loader::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys_loader::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE};
use windows_sys_loader::Win32::System::Threading::GetCurrentProcess;

use crate::proxy::loader::pe::{get_self_base_dir, parse_game_info};

const APPUSER_PATTERN: &[u8] = &[0x83, 0x7C, 0x24, 0x04, 0x00, 0x75];
const APPUSER_BRANCH_OFFSET: usize = 5;

pub unsafe fn apply_appuser(game_base: usize) {
    if game_base == 0 || !is_appuser_bypass_enabled() {
        return;
    }

    let (game_size, _, _, _, _) = parse_game_info(game_base as *mut c_void);
    if game_size == 0 {
        return;
    }

    // SAFETY: Category 3（原始内存）。PE 的 SizeOfImage 覆盖当前已映射的游戏映像，
    // 这里必须复用 AppleChu 原有的整映像 AOB 识别范围，避免改变版本匹配语义。
    let image = unsafe { std::slice::from_raw_parts(game_base as *const u8, game_size as usize) };
    let Some(offset) = find_appuser_branch(image) else {
        return;
    };

    // SAFETY: Category 3（原始内存）。AOB 已确认目标字节为条件跳转 0x75，
    // 地址位于上面校验过的游戏映像范围内。
    unsafe { patch_branch((game_base + offset) as *mut u8) };
}

fn is_appuser_bypass_enabled() -> bool {
    let Some(base_dir) = get_self_base_dir() else {
        return false;
    };
    let config_path = Path::new(&base_dir).join("AppleChu.toml");
    let Ok(text) = std::fs::read_to_string(config_path) else {
        return false;
    };
    appuser_bypass_enabled_in(&text)
}

fn appuser_bypass_enabled_in(text: &str) -> bool {
    let Ok(config) = text.parse::<toml::Table>() else {
        return false;
    };
    config
        .get("BypassAppUser")
        .and_then(toml::Value::as_table)
        .is_some_and(|section| {
            !section
                .get("Disabled")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false)
        })
}

fn find_appuser_branch(image: &[u8]) -> Option<usize> {
    image
        .windows(APPUSER_PATTERN.len())
        .position(|window| window == APPUSER_PATTERN)
        .and_then(|offset| offset.checked_add(APPUSER_BRANCH_OFFSET))
}

unsafe fn patch_branch(address: *mut u8) {
    let mut old_protect = 0;
    if VirtualProtect(address.cast(), 1, PAGE_EXECUTE_READWRITE, &mut old_protect) == 0 {
        return;
    }

    address.write(0xEB);

    let mut ignored = 0;
    let _ = VirtualProtect(address.cast(), 1, old_protect, &mut ignored);
    let _ = FlushInstructionCache(GetCurrentProcess(), address.cast(), 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_section_controls_early_patch() {
        assert!(appuser_bypass_enabled_in("[BypassAppUser]"));
        assert!(appuser_bypass_enabled_in(
            "[BypassAppUser]\nDisabled = false"
        ));
        assert!(!appuser_bypass_enabled_in(
            "[BypassAppUser]\nDisabled = true"
        ));
        assert!(!appuser_bypass_enabled_in("[DisableTLS]"));
        assert!(!appuser_bypass_enabled_in("not toml"));
    }

    #[test]
    fn existing_aob_selects_first_matching_branch() {
        let mut image = vec![0x90; 32];
        image[3..9].copy_from_slice(APPUSER_PATTERN);
        image[20..26].copy_from_slice(APPUSER_PATTERN);

        assert_eq!(find_appuser_branch(&image), Some(8));
    }
}
