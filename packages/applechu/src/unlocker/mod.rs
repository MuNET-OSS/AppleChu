// 感谢 @MoeGrid 提供的解锁模块
//! XML 文件解锁模块
//!
//! 通过 iohook 责任链拦截文件打开，在内存中替换 XML 内容实现 CHUNITHM
//! 内容解锁，不修改磁盘上的原始文件。
//!
//! 原理参照 chunlocker：检测到游戏以只读方式打开目标 XML 时，读取原始文件，
//! 在内存中改写标签值（如 `<defaultHave>false</defaultHave>` →
//! `<defaultHave>true</defaultHave>`），再用 NUL 设备句柄作为占位返回，
//! 后续 Read / Seek / GetFileSize 全部由本模块从内存缓冲区应答。
//!
//! 本模块读取原始文件用的 `std::fs::read` 同样会经过 iohook 责任链，因此
//! VFS 路径（`C:\Mount\Option\...` 等）会由 `platform::path_hook` 正常转换；
//! 重入标志保证不会递归回到本模块。

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use once_cell::sync::Lazy;

use crate::iohook::{self, Irp, IrpOp};
use crate::platform::winapi;
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat_all_modules;
use crate::util::win32::handle_value;

// Win32 错误码
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NEGATIVE_SEEK: u32 = 131;
const INVALID_FILE_SIZE: u32 = 0xFFFF_FFFF;

// CreateFile 参数位
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const FILE_WRITE_DATA: u32 = 0x0002;
const FILE_APPEND_DATA: u32 = 0x0004;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;

// SetFilePointer 定位基准
const FILE_BEGIN: u32 = 0;
const FILE_CURRENT: u32 = 1;
const FILE_END: u32 = 2;

// ---------------------------------------------------------------------------
// 配置
// ---------------------------------------------------------------------------

crate::config_section! {
    pub(crate) struct UnlockerConfig => UNLOCKER_CONFIG_SECTION {
        section: "Unlocker",
        order: 400,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "gameplay",
        community: true,
        description: "在内存中解锁内容，不修改资源文件",
        description_en: "Unlocks content in memory without modifying resource files",
        comment: "解锁游戏内容（内存改写 XML，不修改磁盘文件）",
        fields: {
            pub unlock_chara: bool = true,
            comment: "解锁角色（Chara.xml）";

            pub unlock_music: bool = true,
            comment: "解锁乐曲（Music.xml）";

            pub unlock_nameplate: bool = true,
            key: "unlockNamePlate",
            comment: "解锁铭牌（NamePlate.xml）";

            pub unlock_systemvoice: bool = true,
            key: "unlockSystemVoice",
            comment: "解锁系统语音（SystemVoice.xml）";

            pub unlock_event: bool = true,
            comment: "解锁活动（Event.xml）";

            pub unlock_mapicon: bool = true,
            key: "unlockMapIcon",
            comment: "解锁跑图小人（MapIcon.xml）";

            pub unlock_trophy: bool = true,
            comment: "解锁称号（Trophy.xml）";
        }
    }
}

// ---------------------------------------------------------------------------
// 解锁规则
// ---------------------------------------------------------------------------

/// 单条解锁规则：文件名 → 需要改写的 XML 标签及目标值。
struct UnlockRule {
    /// 目标文件名（不含目录，忽略大小写）
    filename: &'static str,
    /// 日志中显示的中文名
    label: &'static str,
    /// 需要改写的标签名
    tag: &'static str,
    /// 标签的目标值
    value: &'static str,
    /// 该规则对应的配置开关
    enabled: fn(&UnlockerConfig) -> bool,
}

const RULES: &[UnlockRule] = &[
    UnlockRule {
        filename: "Chara.xml",
        label: "character",
        tag: "defaultHave",
        value: "true",
        enabled: |config| config.unlock_chara,
    },
    UnlockRule {
        filename: "Music.xml",
        label: "music",
        tag: "firstLock",
        value: "false",
        enabled: |config| config.unlock_music,
    },
    UnlockRule {
        filename: "NamePlate.xml",
        label: "nameplate",
        tag: "defaultHave",
        value: "true",
        enabled: |config| config.unlock_nameplate,
    },
    UnlockRule {
        filename: "SystemVoice.xml",
        label: "system voice",
        tag: "defaultHave",
        value: "true",
        enabled: |config| config.unlock_systemvoice,
    },
    UnlockRule {
        filename: "Event.xml",
        label: "event",
        tag: "alwaysOpen",
        value: "true",
        enabled: |config| config.unlock_event,
    },
    UnlockRule {
        filename: "MapIcon.xml",
        label: "map icon",
        tag: "defaultHave",
        value: "true",
        enabled: |config| config.unlock_mapicon,
    },
    UnlockRule {
        filename: "Trophy.xml",
        label: "trophy",
        tag: "defaultHave",
        value: "true",
        enabled: |config| config.unlock_trophy,
    },
];

// 日志去重用 u32 位图按规则下标标记，位宽必须覆盖规则数量。
const _: () = assert!(RULES.len() <= u32::BITS as usize);

/// 查找与文件名匹配且已启用的解锁规则，返回规则下标。
fn find_rule(config: &UnlockerConfig, filename: &str) -> Option<usize> {
    // CreateFile 是热路径，先用扩展名快速否决绝大多数调用。
    if !has_xml_extension(filename) {
        return None;
    }
    RULES
        .iter()
        .position(|rule| filename.eq_ignore_ascii_case(rule.filename) && (rule.enabled)(config))
}

fn has_xml_extension(filename: &str) -> bool {
    filename
        .len()
        .checked_sub(4)
        .and_then(|split| filename.get(split..))
        .is_some_and(|ext| ext.eq_ignore_ascii_case(".xml"))
}

// ---------------------------------------------------------------------------
// 句柄状态管理
// ---------------------------------------------------------------------------

/// 每个被拦截的文件句柄对应一份修改后的内容和当前读取位置。
struct FileState {
    data: Vec<u8>,
    pos: usize,
}

static UNLOCKER_CONFIG: OnceLock<UnlockerConfig> = OnceLock::new();
static UNLOCKER_FILES: Lazy<Mutex<HashMap<usize, FileState>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn is_unlocker_handle(handle: usize) -> bool {
    UNLOCKER_FILES
        .lock()
        .ok()
        .is_some_and(|map| map.contains_key(&handle))
}

fn unlocker_file_size(handle: usize) -> u64 {
    UNLOCKER_FILES
        .lock()
        .ok()
        .and_then(|map| map.get(&handle).map(|state| state.data.len() as u64))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 重入保护
// ---------------------------------------------------------------------------

// 线程本地重入标志。
//
// `load_modified` 用 `std::fs::read` 读取原始文件，Rust 标准库内部会调用
// `CreateFileW` / `ReadFile`；这些调用经过 iohook 的 IAT hook 会再次进入
// 本模块的 handler。持有 guard 期间重入直接放行，避免无限递归。
//
// 注意：正因为嵌套调用仍会走完 iohook 责任链，我们的 `std::fs::read` 才能
// 落到 `path_hook` 的 CreateFileW 上并享受 VFS 路径转换 —— 游戏传进来的
// `C:\Mount\Option\...` 之类虚拟路径因此能被正确解析到真实文件。
thread_local! {
    static IN_UNLOCKER: Cell<bool> = const { Cell::new(false) };
}

struct ReentrancyGuard;

impl ReentrancyGuard {
    /// 未重入时返回 guard 并置位标志；已在模块内部时返回 None。
    fn acquire() -> Option<Self> {
        IN_UNLOCKER.with(|flag| {
            if flag.get() {
                None
            } else {
                flag.set(true);
                Some(Self)
            }
        })
    }
}

impl Drop for ReentrancyGuard {
    fn drop(&mut self) {
        IN_UNLOCKER.with(|flag| flag.set(false));
    }
}

// ---------------------------------------------------------------------------
// GetFileSize / GetFileSizeEx hook
// ---------------------------------------------------------------------------

/// iohook 框架未 hook GetFileSize，但游戏可能在 ReadFile 前调用它来
/// 获取文件大小并分配缓冲区。由于我们使用 NUL 设备句柄作为占位，
/// 原始 GetFileSize 对 NUL 返回 0，会导致游戏认为文件为空。
/// 因此在此模块单独 hook 这两个 API，对被拦截的句柄返回修改后的内容长度。
static ORIG_GET_FILE_SIZE: AtomicUsize = AtomicUsize::new(0);
static ORIG_GET_FILE_SIZE_EX: AtomicUsize = AtomicUsize::new(0);

type GetFileSizeFn = unsafe extern "system" fn(usize, *mut u32) -> u32;
type GetFileSizeExFn = unsafe extern "system" fn(usize, *mut i64) -> i32;

unsafe fn install_file_size_hooks(api: &Api) {
    if let Some(original) = hook_iat_all_modules(
        "kernel32.dll",
        "GetFileSize",
        hooked_get_file_size as *const (),
    ) {
        ORIG_GET_FILE_SIZE.store(original as usize, Ordering::SeqCst);
        api.log_info("Unlocker installed GetFileSize hook");
    }

    if let Some(original) = hook_iat_all_modules(
        "kernel32.dll",
        "GetFileSizeEx",
        hooked_get_file_size_ex as *const (),
    ) {
        ORIG_GET_FILE_SIZE_EX.store(original as usize, Ordering::SeqCst);
        api.log_info("Unlocker installed GetFileSizeEx hook");
    }
}

unsafe extern "system" fn hooked_get_file_size(handle: usize, high: *mut u32) -> u32 {
    if is_unlocker_handle(handle) {
        let size = unlocker_file_size(handle);
        if !high.is_null() {
            *high = (size >> 32) as u32;
        }
        return size as u32;
    }
    let addr = ORIG_GET_FILE_SIZE.load(Ordering::SeqCst);
    if addr == 0 {
        iohook::set_last_error(ERROR_INVALID_HANDLE);
        return INVALID_FILE_SIZE;
    }
    let original: GetFileSizeFn = std::mem::transmute(addr);
    original(handle, high)
}

unsafe extern "system" fn hooked_get_file_size_ex(handle: usize, size: *mut i64) -> i32 {
    if is_unlocker_handle(handle) {
        if !size.is_null() {
            *size = unlocker_file_size(handle) as i64;
        }
        return 1;
    }
    let addr = ORIG_GET_FILE_SIZE_EX.load(Ordering::SeqCst);
    if addr == 0 {
        iohook::set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    }
    let original: GetFileSizeExFn = std::mem::transmute(addr);
    original(handle, size)
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

// stage = IoHook：本模块依赖 iohook 的 CreateFile/ReadFile 责任链，
// 与设备仿真无关。order 必须大于 iohook::init_all（order = 10），
// 否则 push_handler 会在钩子安装前执行。
#[applechu_macros::config_section(stage = IoHook, order = 20)]
pub fn init(api: &Api, config: &UnlockerConfig) {
    let enabled: Vec<&str> = RULES
        .iter()
        .filter(|rule| (rule.enabled)(config))
        .map(|rule| rule.filename)
        .collect();

    if enabled.is_empty() {
        api.log_warn("Unlocker has no enabled content types; skipping initialization");
        return;
    }

    let _ = UNLOCKER_CONFIG.set(config.clone());

    unsafe {
        install_file_size_hooks(api);
        iohook::push_handler(unlocker_irp_handler);
    }

    api.log_info(&format!(
        "Unlocker initialized (in-memory XML replacement): [{}]",
        enabled.join(", ")
    ));
}

// ---------------------------------------------------------------------------
// IRP handler
// ---------------------------------------------------------------------------

unsafe fn unlocker_irp_handler(irp: &mut Irp) -> i32 {
    if irp.op == IrpOp::Open {
        return handle_open(irp);
    }

    let handle_val = handle_value(irp.fd);
    if !is_unlocker_handle(handle_val) {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => handle_close(irp, handle_val),
        IrpOp::Read => handle_read(irp, handle_val),
        IrpOp::Seek => handle_seek(irp, handle_val),
        IrpOp::Write => {
            // 写操作静默丢弃：正常情况到不了这里（handle_open 只拦截只读打开），
            // 保留作为兜底，避免把游戏的写入落到 NUL 之外的地方。
            if !irp.out_nbytes.is_null() {
                *irp.out_nbytes = irp.nbytes;
            }
            1
        }
        // Open 已在上方处理；Fsync 对内存缓冲无意义，直接报成功。
        IrpOp::Fsync | IrpOp::Open => 1,
        IrpOp::Ioctl => {
            iohook::set_last_error(ERROR_INVALID_FUNCTION);
            0
        }
    }
}

/// 处理文件打开：匹配目标 XML → 读取原始内容 → 内存修改 → 返回虚拟句柄
unsafe fn handle_open(irp: &mut Irp) -> i32 {
    let Some(config) = UNLOCKER_CONFIG.get() else {
        return iohook::invoke_next(irp);
    };

    // 只接管只读、同步打开。写入 / 截断 / 创建 / 重叠 I/O 一律放行，
    // 否则游戏的写操作会被 NUL 句柄静默吞掉。
    if !is_read_only_open(irp) {
        return iohook::invoke_next(irp);
    }

    let Some(path) = open_path(irp) else {
        return iohook::invoke_next(irp);
    };

    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");

    let Some(rule_index) = find_rule(config, filename) else {
        return iohook::invoke_next(irp);
    };

    // 读盘 + 改写在独立函数里完成，重入 guard 随之释放，
    // 不会覆盖到下面的 invoke_next。
    let Some(modified) = load_modified(&path, filename, rule_index) else {
        return iohook::invoke_next(irp);
    };

    let Some(fd) = iohook::open_nul_fd() else {
        return iohook::invoke_next(irp);
    };
    let handle_val = handle_value(fd);
    let modified_len = modified.len();

    if let Ok(mut files) = UNLOCKER_FILES.lock() {
        files.insert(
            handle_val,
            FileState {
                data: modified,
                pos: 0,
            },
        );
    } else {
        return iohook::invoke_next(irp);
    }

    log_rule_once(rule_index, modified_len);
    irp.fd = fd;
    1
}

/// 读取原始文件并按规则改写；返回 `None` 表示应放行读原始文件。
///
/// 重入保护只覆盖这段：`std::fs::read` 内部的 `CreateFileW` 会经 IAT
/// 再次进入 `handle_open`，若不设标志会对同名文件无限递归。
/// 注意此处依赖同一条责任链：我们的 `CreateFileW` 仍会落到
/// `path_hook`，因此 `C:\Mount\Option\...` 这类 VFS 路径能正确解析。
fn load_modified(path: &str, filename: &str, rule_index: usize) -> Option<Vec<u8>> {
    let _guard = ReentrancyGuard::acquire()?;
    let rule = &RULES[rule_index];

    let raw = match std::fs::read(path) {
        Ok(data) => data,
        Err(error) => {
            warn_once(rule_index, &LOGGED_READ_FAIL, || {
                format!("Unlocker could not read {}: {}", filename, error)
            });
            return None;
        }
    };

    // 非 UTF-8 文件（UTF-16、含非法字节等）直接放行：
    // from_utf8_lossy 会把非法字节替换成 U+FFFD，等于把文件喂坏。
    let Ok(text) = std::str::from_utf8(&raw) else {
        warn_once(rule_index, &LOGGED_NOT_UTF8, || {
            format!(
                "Unlocker found invalid UTF-8 in {}; using the original file",
                filename
            )
        });
        return None;
    };

    let Some(modified) = modify_xml(text, rule.tag, rule.value) else {
        // 同名文件有上千个（每首曲子 / 每个角色一份），其中一部分的标签
        // 本来就是目标值或压根没这个标签，无需改写 —— 属正常情况，不是故障。
        info_once(rule_index, &LOGGED_NO_TAG, || {
            format!(
                "Unlocker found no required <{}> change in {}; using the original file",
                rule.tag, filename
            )
        });
        return None;
    };

    Some(modified.into_bytes())
}

/// 处理文件关闭：先移除状态，再让责任链关闭 NUL 句柄。
///
/// 顺序不能反：若先关闭句柄，该值可能被其它线程立刻复用，
/// 随后的 remove 就会删掉别人的条目。
unsafe fn handle_close(irp: &mut Irp, handle_val: usize) -> i32 {
    if let Ok(mut files) = UNLOCKER_FILES.lock() {
        files.remove(&handle_val);
    }
    iohook::invoke_next(irp)
}

/// 处理文件读取：从改写后的缓冲区按位置拷贝数据
unsafe fn handle_read(irp: &mut Irp, handle_val: usize) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }

    let Ok(mut files) = UNLOCKER_FILES.lock() else {
        iohook::set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    };
    let Some(state) = files.get_mut(&handle_val) else {
        iohook::set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    };

    // 同步句柄上传入 OVERLAPPED 是合法的，此时偏移由结构体给出而非当前位置。
    let overlapped = irp.ovl.cast::<Overlapped>();
    let start = if overlapped.is_null() {
        state.pos
    } else {
        let offset = (*overlapped).offset as u64 | (((*overlapped).offset_high as u64) << 32);
        offset.min(usize::MAX as u64) as usize
    };

    let available = state.data.len().saturating_sub(start);
    let count = available.min(irp.nbytes as usize);

    if count > 0 && !irp.read_buf.is_null() {
        std::ptr::copy_nonoverlapping(state.data.as_ptr().add(start), irp.read_buf, count);
    } else if count > 0 {
        iohook::set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }

    if overlapped.is_null() {
        state.pos = start + count;
    } else {
        (*overlapped).internal = 0;
        (*overlapped).internal_high = count;
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = count as u32;
    }
    1
}

/// 处理文件定位：在虚拟缓冲区上模拟 Seek
unsafe fn handle_seek(irp: &mut Irp, handle_val: usize) -> i32 {
    const FILE_BEGIN: u32 = 0;
    const FILE_CURRENT: u32 = 1;
    const FILE_END: u32 = 2;

    let Ok(mut files) = UNLOCKER_FILES.lock() else {
        iohook::set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    };
    let Some(state) = files.get_mut(&handle_val) else {
        iohook::set_last_error(ERROR_INVALID_HANDLE);
        return 0;
    };

    let base = match irp.seek_method {
        FILE_BEGIN => 0i64,
        FILE_CURRENT => state.pos as i64,
        FILE_END => state.data.len() as i64,
        _ => {
            iohook::set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
    };

    // 定位到负偏移是错误，不是钳到 0；越过 EOF 则合法（读取时自然返回 0 字节）。
    let Some(new_pos) = base.checked_add(irp.seek_distance).filter(|pos| *pos >= 0) else {
        iohook::set_last_error(ERROR_NEGATIVE_SEEK);
        return 0;
    };

    state.pos = new_pos as usize;
    if !irp.seek_result.is_null() {
        *irp.seek_result = new_pos;
    }
    1
}

// ---------------------------------------------------------------------------
// 打开参数判定与路径解析
// ---------------------------------------------------------------------------

/// 仅当打开方式是「只读、不创建、不截断、同步」时才接管。
fn is_read_only_open(irp: &Irp) -> bool {
    const WRITE_BITS: u32 = GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA | FILE_APPEND_DATA;
    irp.open_access & WRITE_BITS == 0
        && matches!(irp.open_creation, OPEN_EXISTING | OPEN_ALWAYS)
        && irp.open_flags & FILE_FLAG_OVERLAPPED == 0
}

unsafe fn open_path(irp: &Irp) -> Option<String> {
    unsafe { winapi::cstr_to_string(irp.open_filename_a.cast()) }
        .or_else(|| unsafe { winapi::wide_to_string(irp.open_filename_w) })
}

/// 与 Win32 `OVERLAPPED` 布局一致；windows-sys 的定义带联合体，
/// 这里只取需要的偏移字段。
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut c_void,
}

// ---------------------------------------------------------------------------
// XML 内容改写
// ---------------------------------------------------------------------------

/// 把 XML 中所有指定标签的文本内容替换为目标值，返回 `None` 表示无需改动。
///
/// 例如 `tag = "defaultHave"`, `target = "true"` 会把
/// `<defaultHave>false</defaultHave>` 改成 `<defaultHave>true</defaultHave>`。
///
/// 支持带属性的开标签 `<defaultHave id="1">false</defaultHave>`；
/// 跳过自闭合标签 `<defaultHave/>`；不匹配 `<defaultHaveExtra>` 这类前缀同名标签。
fn modify_xml(text: &str, tag: &str, target: &str) -> Option<String> {
    let open_prefix = format!("<{}", tag);
    let close_tag = format!("</{}>", tag);

    let mut result = String::with_capacity(text.len());
    let mut search_from = 0;
    let mut changed = false;

    loop {
        let Some(rel) = text[search_from..].find(&open_prefix) else {
            result.push_str(&text[search_from..]);
            break;
        };
        let abs = search_from + rel;

        // 精确标签名匹配：`<tag` 之后必须是合法的 XML 分隔符，
        // 否则 `<defaultHave` 会误匹配 `<defaultHaveExtra>`。
        let after_name = abs + open_prefix.len();
        let exact = text
            .as_bytes()
            .get(after_name)
            .is_some_and(|&byte| matches!(byte, b'>' | b'/' | b' ' | b'\t' | b'\n' | b'\r'));
        if !exact {
            result.push_str(&text[search_from..after_name]);
            search_from = after_name;
            continue;
        }

        // 定位开标签的 '>'
        let Some(gt_rel) = text[after_name..].find('>') else {
            result.push_str(&text[search_from..]);
            break;
        };
        let open_end = after_name + gt_rel;

        // 自闭合标签 <tag .../> 没有文本内容，原样保留
        if text[after_name..open_end].ends_with('/') {
            result.push_str(&text[search_from..=open_end]);
            search_from = open_end + 1;
            continue;
        }

        // 定位闭合标签
        let Some(close_rel) = text[open_end + 1..].find(&close_tag) else {
            result.push_str(&text[search_from..=open_end]);
            search_from = open_end + 1;
            continue;
        };
        let inner = &text[open_end + 1..open_end + 1 + close_rel];
        let close_end = open_end + 1 + close_rel + close_tag.len();

        // 开标签（含属性）+ 目标值 + 闭合标签
        result.push_str(&text[search_from..=open_end]);
        result.push_str(target);
        result.push_str(&close_tag);
        if inner != target {
            changed = true;
        }

        search_from = close_end;
    }

    changed.then_some(result)
}

// ---------------------------------------------------------------------------
// 日志去重
// ---------------------------------------------------------------------------

// Chunithm 的 Chara.xml / Music.xml 是每个条目一个文件，量级上千。
// 每类日志用一张位图按规则下标去重，只输出首次命中，避免刷满日志。
static LOGGED_INTERCEPT: AtomicU32 = AtomicU32::new(0);
static LOGGED_READ_FAIL: AtomicU32 = AtomicU32::new(0);
static LOGGED_NOT_UTF8: AtomicU32 = AtomicU32::new(0);
static LOGGED_NO_TAG: AtomicU32 = AtomicU32::new(0);

/// 首次为该规则置位时返回 true。
fn claim_log_slot(seen: &AtomicU32, rule_index: usize) -> bool {
    let bit = 1u32 << rule_index;
    seen.fetch_or(bit, Ordering::Relaxed) & bit == 0
}

/// 该规则首次触发时输出一行 warn，后续静默。用于真正的异常（读盘失败、编码不符）。
fn warn_once(rule_index: usize, seen: &AtomicU32, message: impl FnOnce() -> String) {
    if !claim_log_slot(seen, rule_index) {
        return;
    }
    if let Some(api) = crate::util::api::API.get() {
        api.log_warn(&message());
    }
}

/// 该规则首次触发时输出一行 info，后续静默。用于正常但值得知道的情况。
fn info_once(rule_index: usize, seen: &AtomicU32, message: impl FnOnce() -> String) {
    if !claim_log_slot(seen, rule_index) {
        return;
    }
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(&message());
    }
}

/// 该规则首次拦截成功时输出一行 info，后续静默。
fn log_rule_once(rule_index: usize, size: usize) {
    if !claim_log_slot(&LOGGED_INTERCEPT, rule_index) {
        return;
    }
    let rule = &RULES[rule_index];
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(&format!(
            "Unlocker applied {} rule to {} (<{}> = {}, {} bytes)",
            rule.label, rule.filename, rule.tag, rule.value, size
        ));
    }
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{find_rule, has_xml_extension, modify_xml, UnlockerConfig, RULES};

    #[test]
    fn replaces_simple_tag() {
        let output = modify_xml(
            "<root><defaultHave>false</defaultHave></root>",
            "defaultHave",
            "true",
        );
        assert_eq!(
            output.as_deref(),
            Some("<root><defaultHave>true</defaultHave></root>")
        );
    }

    #[test]
    fn replaces_every_occurrence() {
        let input =
            "<r><i><defaultHave>false</defaultHave></i><i><defaultHave>false</defaultHave></i></r>";
        let output = modify_xml(input, "defaultHave", "true");
        assert_eq!(
            output.as_deref(),
            Some("<r><i><defaultHave>true</defaultHave></i><i><defaultHave>true</defaultHave></i></r>")
        );
    }

    #[test]
    fn preserves_attributes_on_open_tag() {
        let output = modify_xml(
            "<root><defaultHave id=\"1\">false</defaultHave></root>",
            "defaultHave",
            "true",
        );
        assert_eq!(
            output.as_deref(),
            Some("<root><defaultHave id=\"1\">true</defaultHave></root>")
        );
    }

    #[test]
    fn skips_self_closing_tag() {
        assert_eq!(
            modify_xml("<root><defaultHave/></root>", "defaultHave", "true"),
            None
        );
    }

    #[test]
    fn skips_prefixed_tag_name() {
        // <defaultHaveExtra> 不该被 <defaultHave> 匹配到
        assert_eq!(
            modify_xml(
                "<root><defaultHaveExtra>x</defaultHaveExtra></root>",
                "defaultHave",
                "true"
            ),
            None
        );
    }

    #[test]
    fn replaces_firstlock_tag() {
        let output = modify_xml(
            "<root><firstLock>true</firstLock></root>",
            "firstLock",
            "false",
        );
        assert_eq!(
            output.as_deref(),
            Some("<root><firstLock>false</firstLock></root>")
        );
    }

    #[test]
    fn returns_none_when_tag_absent() {
        assert_eq!(
            modify_xml("<root><other>v</other></root>", "defaultHave", "true"),
            None
        );
    }

    #[test]
    fn returns_none_when_already_unlocked() {
        // 值已经是目标值，没必要拦截这个句柄
        assert_eq!(
            modify_xml(
                "<root><defaultHave>true</defaultHave></root>",
                "defaultHave",
                "true"
            ),
            None
        );
    }

    #[test]
    fn fills_empty_inner_text() {
        let output = modify_xml(
            "<root><defaultHave></defaultHave></root>",
            "defaultHave",
            "true",
        );
        assert_eq!(
            output.as_deref(),
            Some("<root><defaultHave>true</defaultHave></root>")
        );
    }

    #[test]
    fn keeps_whitespace_inside_open_tag() {
        let output = modify_xml(
            "<root><defaultHave  >false</defaultHave></root>",
            "defaultHave",
            "true",
        );
        assert_eq!(
            output.as_deref(),
            Some("<root><defaultHave  >true</defaultHave></root>")
        );
    }

    #[test]
    fn leaves_unclosed_tag_alone() {
        assert_eq!(
            modify_xml("<root><defaultHave>false", "defaultHave", "true"),
            None
        );
    }

    #[test]
    fn whitespace_around_value_counts_as_change() {
        // 内部文本 " true " != "true"，改写后收紧为 "true"
        let output = modify_xml(
            "<root><defaultHave> true </defaultHave></root>",
            "defaultHave",
            "true",
        );
        assert_eq!(
            output.as_deref(),
            Some("<root><defaultHave>true</defaultHave></root>")
        );
    }

    #[test]
    fn detects_xml_extension_case_insensitively() {
        assert!(has_xml_extension("Chara.xml"));
        assert!(has_xml_extension("chara.XML"));
        assert!(!has_xml_extension("Chara.xml.bak"));
        assert!(!has_xml_extension("xml"));
        assert!(!has_xml_extension(""));
    }

    #[test]
    fn matches_rule_ignoring_case() {
        let config = UnlockerConfig::default();
        assert_eq!(find_rule(&config, "Chara.xml"), Some(0));
        assert_eq!(find_rule(&config, "MUSIC.XML"), Some(1));
        assert_eq!(find_rule(&config, "Other.xml"), None);
        assert_eq!(find_rule(&config, "Chara.bin"), None);
    }

    #[test]
    fn respects_disabled_rule() {
        let config = UnlockerConfig {
            unlock_chara: false,
            ..UnlockerConfig::default()
        };
        assert_eq!(find_rule(&config, "Chara.xml"), None);
        assert_eq!(find_rule(&config, "Music.xml"), Some(1));
    }

    #[test]
    fn every_rule_targets_an_xml_file() {
        // 规则文件名必须过 has_xml_extension 的快速否决，否则永远匹配不上
        for rule in RULES {
            assert!(has_xml_extension(rule.filename), "{}", rule.filename);
        }
    }
}
