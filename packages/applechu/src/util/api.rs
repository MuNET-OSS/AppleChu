use std::ffi::{c_char, c_void, CString};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::OnceCell;

use super::logging::LogLevel;

pub use chu_abi::{ChuModAPI, ChuModInfo};

#[derive(Clone, Copy)]
pub struct Api {
    raw: Option<NonNull<ChuModAPI>>,
    info: RuntimeInfo,
}

#[derive(Clone, Copy)]
struct RuntimeInfo {
    game_base: usize,
    game_size: u32,
    text_base: usize,
    text_size: u32,
}

// SAFETY: ABI 表在 loader 生命周期内只读，所有跨线程调用均由表内函数自行同步
unsafe impl Send for Api {}
// SAFETY: RuntimeInfo 只含值类型，ABI 表在 loader 生命周期内保持稳定
unsafe impl Sync for Api {}

pub static API: OnceCell<Api> = OnceCell::new();
static STANDALONE_LOGGER: AtomicUsize = AtomicUsize::new(0);

pub type StandaloneLogger = unsafe extern "C" fn(LogLevel, *const c_char);

impl Api {
    /// 从 loader 提供的 ABI 指针创建 API 句柄
    ///
    /// # Safety
    /// `raw` 和 `info` 非空时必须指向有效的 ABI 结构，且 `raw` 必须在返回句柄的整个使用期内保持有效
    pub unsafe fn new(raw: *const ChuModAPI, info: *const ChuModInfo) -> Option<Self> {
        let raw = NonNull::new(raw.cast_mut())?;
        // SAFETY: 调用方保证 info 指向有效的 ChuModInfo
        let info = unsafe { info.as_ref() }?;
        Some(Self {
            raw: Some(raw),
            info: RuntimeInfo {
                game_base: info.game_base,
                game_size: info.game_size,
                text_base: info.text_base,
                text_size: info.text_size,
            },
        })
    }

    /// 为独立的 64 位 AM Daemon 构造 API 句柄
    pub fn standalone(logger: StandaloneLogger) -> Option<Self> {
        // SAFETY: 空模块名按 Win32 契约返回当前进程主模块
        let game_base = unsafe {
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) as usize
        };
        if game_base == 0 {
            return None;
        }
        let dos_lfanew = game_base.checked_add(0x3c)?;
        // SAFETY: Windows 载入的主模块是有效 PE 映像，DOS 头覆盖 e_lfanew 字段
        let nt_offset = unsafe { std::ptr::read_unaligned(dos_lfanew as *const u32) } as usize;
        let nt = game_base.checked_add(nt_offset)?;
        // SAFETY: Windows loader 保证 e_lfanew 指向当前映像内可读的 NT 头
        if unsafe { std::ptr::read_unaligned(nt as *const u32) } != 0x0000_4550 {
            return None;
        }
        let size_of_image = nt.checked_add(0x50)?;
        // SAFETY: 已验证 NT 头签名，PE32+ 可选头覆盖 SizeOfImage 字段
        let game_size = unsafe { std::ptr::read_unaligned(size_of_image as *const u32) };
        if game_size == 0 {
            return None;
        }
        STANDALONE_LOGGER.store(logger as usize, Ordering::Release);
        Some(Self {
            raw: None,
            info: RuntimeInfo {
                game_base,
                game_size,
                text_base: 0,
                text_size: 0,
            },
        })
    }

    pub fn install(self) -> bool {
        API.set(self).is_ok()
    }

    pub fn log_info(&self, msg: &str) {
        self.log(LogLevel::Info, msg, |api| api.log_info);
    }

    pub fn log_warn(&self, msg: &str) {
        self.log(LogLevel::Warn, msg, |api| api.log_warn);
    }

    pub fn log_error(&self, msg: &str) {
        self.log(LogLevel::Error, msg, |api| api.log_error);
    }

    pub fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize {
        let Ok(mask) = CString::new(mask) else {
            return 0;
        };
        self.raw()
            .and_then(|api| api.aob_scan)
            .map_or(0, |func| unsafe {
                func(start, size, pattern.as_ptr(), mask.as_ptr())
            })
    }

    pub fn mem_write(&self, addr: usize, data: &[u8]) -> bool {
        let Ok(size) = u32::try_from(data.len()) else {
            return false;
        };
        self.raw()
            .and_then(|api| api.mem_write)
            .is_some_and(|func| unsafe { func(addr, data.as_ptr().cast(), size) == 0 })
    }

    pub fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool {
        let Ok(size) = u32::try_from(buf.len()) else {
            return false;
        };
        self.raw()
            .and_then(|api| api.mem_read)
            .is_some_and(|func| unsafe { func(addr, buf.as_mut_ptr().cast(), size) == 0 })
    }

    #[allow(dead_code)]
    pub fn mem_fill(&self, addr: usize, value: u8, size: u32) -> bool {
        self.raw()
            .and_then(|api| api.mem_fill)
            .is_some_and(|func| unsafe { func(addr, value, size) == 0 })
    }

    pub fn hook_create(&self, target: usize, detour: usize) -> Option<usize> {
        let mut trampoline = std::ptr::null_mut::<c_void>();
        let created = self
            .raw()
            .and_then(|api| api.hook_create)
            .is_some_and(|func| unsafe {
                func(
                    target as *mut c_void,
                    detour as *mut c_void,
                    &mut trampoline,
                ) == 0
            });
        created.then_some(trampoline as usize)
    }

    pub fn hook_enable(&self, target: usize) -> bool {
        self.raw()
            .and_then(|api| api.hook_enable)
            .is_some_and(|func| unsafe { func(target as *mut c_void) == 0 })
    }

    pub fn hook_disable(&self, target: usize) -> bool {
        self.raw()
            .and_then(|api| api.hook_disable)
            .is_some_and(|func| unsafe { func(target as *mut c_void) == 0 })
    }

    pub fn hook_remove(&self, target: usize) -> bool {
        self.raw()
            .and_then(|api| api.hook_remove)
            .is_some_and(|func| unsafe { func(target as *mut c_void) == 0 })
    }

    pub fn register_present_callback(&self, cb: chu_abi::ChuModPresentCallback) -> bool {
        self.raw()
            .and_then(|api| api.register_present_callback)
            .is_some_and(|func| unsafe { func(Some(cb)) == 0 })
    }

    #[allow(dead_code)]
    pub fn register_reset_callback(&self, cb: chu_abi::ChuModResetCallback) -> bool {
        self.raw()
            .and_then(|api| api.register_reset_callback)
            .is_some_and(|func| unsafe { func(Some(cb)) == 0 })
    }

    pub fn set_frame_lock(&self, fps: u32) -> bool {
        self.raw()
            .and_then(|api| api.set_frame_lock)
            .is_some_and(|func| unsafe { func(fps) == 0 })
    }

    pub fn game_base(&self) -> usize {
        self.info.game_base
    }

    pub fn game_size(&self) -> u32 {
        self.info.game_size
    }

    pub fn text_base(&self) -> usize {
        self.info.text_base
    }

    pub fn text_size(&self) -> u32 {
        self.info.text_size
    }

    fn raw(&self) -> Option<&ChuModAPI> {
        self.raw.map(|raw| unsafe { raw.as_ref() })
    }

    fn log(
        &self,
        level: LogLevel,
        msg: &str,
        select: impl FnOnce(&ChuModAPI) -> Option<unsafe extern "C" fn(*const c_char)>,
    ) {
        if let Some(api) = self.raw() {
            if let Some(func) = select(api) {
                if let Ok(msg) = CString::new(msg) {
                    unsafe { func(msg.as_ptr()) };
                }
            }
            return;
        }
        let Ok(msg) = CString::new(msg) else { return };
        let logger = STANDALONE_LOGGER.load(Ordering::Acquire);
        if logger != 0 {
            unsafe { std::mem::transmute::<usize, StandaloneLogger>(logger)(level, msg.as_ptr()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_info_is_owned_after_crossing_ffi_boundary() {
        // SAFETY: ChuModAPI 的整数、裸指针和 Option<fn> 字段均允许全零值
        let raw = unsafe { std::mem::zeroed::<ChuModAPI>() };
        // Given: loader 提供的运行时信息只在初始化调用期间有效。
        let api = {
            let info = ChuModInfo {
                api_version: 4,
                loader_version: std::ptr::null(),
                game_module: std::ptr::null(),
                game_base: 0x1234,
                game_size: 0x5678,
                text_base: 0x9ABC,
                text_size: 0xDEF0,
                rdata_base: 0,
                rdata_size: 0,
            };

            // When: AppleChu 在 FFI 边界创建 API 句柄。
            unsafe { Api::new(&raw, &info) }.expect("有效指针必须被接受")
        };

        // Then: 句柄不再借用 loader 的临时 ChuModInfo。
        assert_eq!(api.game_base(), 0x1234);
        assert_eq!(api.game_size(), 0x5678);
        assert_eq!(api.text_base(), 0x9ABC);
        assert_eq!(api.text_size(), 0xDEF0);
    }
}
