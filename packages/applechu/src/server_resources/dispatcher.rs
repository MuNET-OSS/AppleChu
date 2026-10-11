//! 原生 API 分发器钩子
//!
//! 对应 AquaMai 的 `Packet::ProcImpl`。游戏在 `projClient` 的收包状态机里
//! 先解密再分发：`sub_9A0AB0` 调 `sub_42FD83`（AES 解密）得到明文响应，
//! 随即调 `sub_99FA60`（本模块挂钩的分发器）把 `(逻辑 API 名, 明文响应)`
//! 交给具体 handler。
//!
//! 挂在这一层相比 WinHTTP 的优势：
//! - 拿到的是**已解密的明文**，不需要自己调用 `gm::Aes`；
//! - 拿到的是**逻辑 API 名**（如 `GetGameSettingApi`），不必从 URL 猜。
//!
//! ## 签名（2.50 实测，唯一匹配）
//!
//! `sub_99FA60` 是 `__thiscall`：`(this, apiName*, response*, out*)`，
//! 前两个字符串参数是 MSVC `std::string`（`_Bx`@0，`_Mysize`@0x10，
//! `_Myres`@0x14；`_Myres >= 16` 时数据在 `_Bx._Ptr`，否则在对象内）。

use crate::util::api::Api;
use crate::util::pattern;

/// `sub_99FA60` 的函数头特征（本二进制唯一；栈 cookie 与 SEH 立刻数按 `??` 处理）
const DISPATCHER_SIGNATURE: &str = concat!(
    "55 8B EC 6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50 ",
    "83 EC 08 53 56 57 A1 ?? ?? ?? ?? 33 C5 50 8D 45 F4 ",
    "64 A3 00 00 00 00 8B F9 8B 45 10 B9"
);

/// MSVC `std::string::_Myres` 达到该值表示数据在堆上
const SSO_CAPACITY: u32 = 16;

type DispatcherFn =
    unsafe extern "thiscall" fn(*mut u8, *mut u8, *mut u8, *mut u8) -> i32;

static ORIGINAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static TARGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 安装分发器钩子，返回是否成功
pub fn install(api: &Api) -> Result<(), String> {
    let target = pattern::scan(api, DISPATCHER_SIGNATURE);
    if target == 0 {
        return Err("client dispatcher signature not found".to_owned());
    }
    let Some(trampoline) = api.hook_create(target, hooked_dispatcher as *const () as usize) else {
        return Err("client dispatcher hook could not be created".to_owned());
    };
    if !api.hook_enable(target) {
        api.hook_remove(target);
        return Err("client dispatcher hook could not be enabled".to_owned());
    }
    ORIGINAL.store(trampoline, std::sync::atomic::Ordering::Release);
    TARGET.store(target, std::sync::atomic::Ordering::Release);
    api.log_info(&format!(
        "ServerResources: client dispatcher hooked at 0x{target:08X}"
    ));
    Ok(())
}

pub fn shutdown() {
    let target = TARGET.swap(0, std::sync::atomic::Ordering::AcqRel);
    if target != 0 {
        if let Some(api) = crate::util::api::API.get() {
            api.hook_disable(target);
            api.hook_remove(target);
        }
    }
    ORIGINAL.store(0, std::sync::atomic::Ordering::Release);
}

/// 读取 MSVC `std::string` 的内容
///
/// # Safety
/// `value` 必须指向有效的 `std::string` 对象
unsafe fn read_std_string(value: *const u8) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let length = std::ptr::read_unaligned(value.add(0x10).cast::<u32>()) as usize;
    let capacity = std::ptr::read_unaligned(value.add(0x14).cast::<u32>());
    if length == 0 {
        return Some(String::new());
    }
    // 上限保护：异常长度不至于越界读取
    if length > 1 << 20 {
        return None;
    }
    let data = if capacity >= SSO_CAPACITY {
        let pointer = std::ptr::read_unaligned(value.cast::<u32>()) as usize;
        if pointer == 0 {
            return None;
        }
        pointer as *const u8
    } else {
        value
    };
    let bytes = std::slice::from_raw_parts(data, length);
    Some(String::from_utf8_lossy(bytes).into_owned())
}

unsafe extern "thiscall" fn hooked_dispatcher(
    this: *mut u8,
    api_name: *mut u8,
    response: *mut u8,
    out: *mut u8,
) -> i32 {
    let original = ORIGINAL.load(std::sync::atomic::Ordering::Acquire);
    if original == 0 {
        return 0;
    }
    let original: DispatcherFn = std::mem::transmute(original);

    // 读逻辑 API 名与响应（失败则原样放行）
    if let (Some(name), Some(body)) = (read_std_string(api_name), read_std_string(response)) {
        crate::server_resources::on_net_response(&name, &body);
    }

    original(this, api_name, response, out)
}
