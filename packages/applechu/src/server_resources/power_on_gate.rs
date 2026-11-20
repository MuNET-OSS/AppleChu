//! 开机闸门：下载期间挂起 PowerOn 状态机
//!
//! 对应 AquaMai 在 `PowerOnProcess.OnUpdate` 上把 state 顶到 255 的做法。
//! 这里改用更稳的方式：挂钩 `projGame::PowerOn` 状态机的每帧更新（虚表
//! `0x19249BC` 的第 0 槽），下载进行中直接返回、不调用原函数，状态机因此
//! 停在同一帧；下载结束（或超时）后恢复调用。
//!
//! 安全性：
//! - 收包分发器与下载都在各自线程，冻结 PowerOn 更新不会阻塞它们；
//! - 设了最长冻结时间，下载卡死也不会把开机永久挂住。

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use crate::util::api::Api;

/// `projGame::PowerOn` 虚表 RVA（IDA: `0x19249BC`）
const POWER_ON_VTABLE_RVA: usize = 0x152_49BC;
/// 状态机每帧更新在虚表中的槽位
const UPDATE_SLOT: usize = 0;
/// 最长冻结时间：超时自动放行，避免下载卡死导致开机永久挂起
const MAX_BLOCK_MS: u32 = 60_000;

/// 原更新函数（trampoline）
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// 被挂钩的更新函数地址
static TARGET: AtomicUsize = AtomicUsize::new(0);
/// 是否正在冻结开机流程
static BLOCKING: AtomicBool = AtomicBool::new(false);
/// 冻结开始时刻（`GetTickCount`）
static BLOCK_STARTED: AtomicU64 = AtomicU64::new(0);
/// 已记录过冻结日志，避免刷屏
static BLOCK_LOGGED: AtomicBool = AtomicBool::new(false);

/// 读取系统启动以来的毫秒数（`GetTickCount`，与游戏同源）
#[cfg(windows)]
fn now_ms() -> u32 {
    extern "system" {
        fn GetTickCount() -> u32;
    }
    unsafe { GetTickCount() }
}

#[cfg(not(windows))]
fn now_ms() -> u32 {
    0
}

type UpdateFn = unsafe extern "thiscall" fn(*mut u8);

/// 安装闸门；须在 VFS / 脚本初始化之后的 `Late` 阶段调用
pub fn install(api: &Api) -> Result<(), String> {
    let base = api.game_base();
    if base == 0 {
        return Err("game base is not available".to_owned());
    }
    let vtable = base
        .checked_add(POWER_ON_VTABLE_RVA)
        .ok_or_else(|| "PowerOn vtable address overflowed".to_owned())?;

    let mut slot = [0u8; 4];
    if !api.mem_read(vtable + UPDATE_SLOT * 4, &mut slot) {
        return Err("PowerOn update slot could not be read".to_owned());
    }
    let update = u32::from_le_bytes(slot) as usize;
    let (text_base, text_size) = (api.text_base(), api.text_size() as usize);
    if update == 0 || !(text_base..text_base + text_size).contains(&update) {
        return Err(format!(
            "PowerOn update slot 0x{update:08X} is outside the code segment"
        ));
    }

    let Some(trampoline) = api.hook_create(update, hooked_update as *const () as usize) else {
        return Err("PowerOn update hook could not be created".to_owned());
    };
    if !api.hook_enable(update) {
        api.hook_remove(update);
        return Err("PowerOn update hook could not be enabled".to_owned());
    }
    ORIGINAL.store(trampoline, Ordering::Release);
    TARGET.store(update, Ordering::Release);
    api.log_info(&format!(
        "ServerResources: boot gate armed on PowerOn update at 0x{update:08X}"
    ));
    Ok(())
}

pub fn shutdown() {
    BLOCKING.store(false, Ordering::Release);
    let target = TARGET.swap(0, Ordering::AcqRel);
    if target != 0 {
        if let Some(api) = crate::util::api::API.get() {
            api.hook_disable(target);
            api.hook_remove(target);
        }
    }
    ORIGINAL.store(0, Ordering::Release);
    BLOCK_LOGGED.store(false, Ordering::Release);
}

/// 开始冻结开机流程
pub fn begin_block() {
    BLOCK_STARTED.store(u64::from(now_ms()), Ordering::Release);
    BLOCKING.store(true, Ordering::Release);
    if !BLOCK_LOGGED.swap(true, Ordering::AcqRel) {
        log_info("ServerResources: boot held while downloading server resources");
    }
}

/// 结束冻结，恢复开机流程
pub fn end_block() {
    if BLOCKING.swap(false, Ordering::Release) {
        log_info("ServerResources: boot resumed");
    }
    BLOCK_LOGGED.store(false, Ordering::Release);
}

/// 是否处于冻结中（含超时判定）
fn blocking() -> bool {
    if !BLOCKING.load(Ordering::Acquire) {
        return false;
    }
    let started = BLOCK_STARTED.load(Ordering::Acquire);
    if started == 0 {
        return false;
    }
    let now = u64::from(now_ms());
    if now.wrapping_sub(started) > u64::from(MAX_BLOCK_MS) {
        // 超时自动放行，避免下载异常时开机永久挂起
        BLOCKING.store(false, Ordering::Release);
        log_warn("ServerResources: boot gate timed out, resuming boot");
        return false;
    }
    true
}

unsafe extern "thiscall" fn hooked_update(this: *mut u8) {
    if blocking() {
        // 停在当前帧：不推进状态机，也不调用原函数
        return;
    }
    let original = ORIGINAL.load(Ordering::Acquire);
    if original == 0 {
        return;
    }
    let original: UpdateFn = std::mem::transmute(original);
    original(this);
}

fn log_info(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
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
    fn gate_is_open_by_default() {
        // Given: 未开始冻结。
        // Then: 不阻塞开机，超时判定也不会误触发。
        assert!(!blocking());
    }

    #[test]
    fn block_and_release_round_trip() {
        // Given: 开始冻结。
        begin_block();

        // When/Then: 冻结生效，释放后恢复。
        assert!(blocking());
        end_block();
        assert!(!blocking());
    }

    #[test]
    fn stale_start_time_opens_the_gate() {
        // Given: 冻结但开始时刻被设为 0（异常状态）。
        BLOCKING.store(true, Ordering::Release);
        BLOCK_STARTED.store(0, Ordering::Release);

        // Then: 判定为不阻塞，避免永久挂起。
        assert!(!blocking());
        BLOCKING.store(false, Ordering::Release);
    }
}
