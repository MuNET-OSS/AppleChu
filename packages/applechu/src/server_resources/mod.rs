//! 服务器下发自定义 Option 资源
//!
//! 架构对齐 AquaMai `ServerResources`：监听游戏正常的 `GetGameSettingApi`
//! 响应 → 读 `_appleChuResources` → 条件筛选 → 下载 → RSA 验签 → 缓存 →
//! 解压为磁盘上的 option 目录树 → 交给 VFS 与游戏本身。
//!
//! 缓存只保留资源包本体（与 AquaMai 落一个缓存文件一致，不进游戏目录）；
//! 解压出的 `AXXX` 树由 `platform/vfs` 重定向呈现，读取与子目录枚举都走游戏
//! 原有的文件 API，本功能只补 option 根那一级枚举（见 `vfs_bridge`）。
//!
//! 流程细节见 `D:\Git\work\plan\服务器下发-架构.md`。

pub mod deflate;
pub mod dispatcher;
pub mod download;
pub mod entry;
pub mod json;
pub mod net_packet;
pub mod package;
pub mod power_on_gate;
pub mod vfs_bridge;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::config::Config;
use crate::platform::vfs;
use crate::server_resources::json::JsonValue;
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct ServerResourcesConfig => SERVER_RESOURCES_CONFIG_SECTION {
        section: "ServerResources",
        order: 355,
        default_on: false,
        always_enabled: false,
        hidden: false,
        group: "network",
        comment: "服务器下发自定义 Option 资源",
        fields: {
            pub public_key: String = String::new(),
            advanced: true,
            comment: "RSA 公钥（base64 modulus），留空则忽略服务器下发";
            pub cache_path: String = String::from("tmp/ServerResources"),
            advanced: true,
            comment: "资源包缓存路径（相对游戏目录）";
            pub native_dispatcher: bool = true,
            advanced: true,
            comment: "挂在原生收包分发器上；关闭则退回 WinHTTP 响应层";
            pub hold_boot: bool = true,
            advanced: true,
            comment: "下载期间挂起开机流程";
        }
    }
}

/// 已挂载的自定义 option 根
struct MountedOption {
    /// 根目录名（`AXXX`，保留包内原始大小写）
    root: String,
    /// 解压目录绝对路径
    directory: PathBuf,
}

/// 当前生效的自定义 option；供枚举注入追加到 option 根
static MOUNTED: OnceLock<Mutex<Vec<MountedOption>>> = OnceLock::new();
static DOWNLOAD_STARTED: AtomicBool = AtomicBool::new(false);

fn mounted() -> &'static Mutex<Vec<MountedOption>> {
    MOUNTED.get_or_init(|| Mutex::new(Vec::new()))
}

/// 已挂载的自定义 option 根目录名（按挂载顺序）
pub(crate) fn mounted_roots() -> Vec<String> {
    mounted()
        .lock()
        .map(|mounted| mounted.iter().map(|entry| entry.root.clone()).collect())
        .unwrap_or_default()
}

pub(crate) fn is_enabled(config: &Config) -> bool {
    config
        .section::<ServerResourcesConfig>()
        .is_some_and(|section| section.enabled)
}

/// 初始化：注册信令处理与枚举注入
#[applechu_macros::config_section(stage = Late, order = 60, shutdown = shutdown)]
pub(crate) fn init(api: &Api, config: &Config) -> Result<(), String> {
    if !is_enabled(config) {
        return Ok(());
    }

    let Some(section) = config.section::<ServerResourcesConfig>() else {
        return Ok(());
    };
    if section.public_key.trim().is_empty() {
        api.log_warn("ServerResources: public_key is empty, resource download is disabled");
        return Ok(());
    }

    // 信令来源：优先挂在原生分发器（拿逻辑 API 名 + 明文响应），
    // 关闭或挂钩失败时退回 WinHTTP 响应层。
    if section.native_dispatcher {
        match dispatcher::install(api) {
            Ok(()) => {}
            Err(error) => {
                api.log_warn(&format!(
                    "ServerResources: native dispatcher unavailable ({error}), falling back to WinHTTP"
                ));
                net_packet::subscribe(on_response);
                net_packet::install(api, config)?;
            }
        }
    } else {
        net_packet::subscribe(on_response);
        net_packet::install(api, config)?;
    }
    vfs_bridge::install(api)?;
    if section.hold_boot {
        // 闸门装不上不致命：退化为「下载不阻塞开机」
        if let Err(error) = power_on_gate::install(api) {
            api.log_warn(&format!("ServerResources: boot gate unavailable: {error}"));
        }
    }
    api.log_info("ServerResources: waiting for GetGameSettingApi response");
    Ok(())
}

/// 关闭：还原钩子并注销已挂载的 option
pub(crate) fn shutdown() {
    power_on_gate::shutdown();
    dispatcher::shutdown();
    vfs_bridge::shutdown();
    if let Ok(mounted) = mounted().lock() {
        for entry in mounted.iter() {
            vfs::set_option_override(&entry.root, "");
        }
    }
}

/// 原生分发器回传的响应：解析 JSON 后进入条件筛选
///
/// 与 WinHTTP 层共用同一段逻辑，区别只在信令来源。
pub(crate) fn on_net_response(api_name: &str, body: &str) {
    let Some(mut value) = json::parse(body) else {
        return;
    };
    // 广播给 chu-abi 模组（与 WinHTTP 路径行为一致）
    net_packet::publish(api_name, &value);
    on_response(api_name, &mut value);
}

/// 信令回调：在 `GetGameSettingApi` 响应中查找下发条目
fn on_response(api_name: &str, response: &mut JsonValue) -> bool {
    // 只监听登录设置响应；其他 API 与包含资源字段的响应都不处理
    if api_name != "GetGameSettingApi" {
        return false;
    }
    if let Some(resources) = locate_resources(response) {
        let entries = entry::parse_entries(resources);
        if entries.is_empty() {
            return false;
        }

        let game_version = game_version_from(response);
        let Some(selected) = entry::select_entry(&entries, None, game_version) else {
            log_info(&format!(
                "ServerResources: {count} entr{suffix} present but none matched (gameVersion={game_version})",
                count = entries.len(),
                suffix = if entries.len() == 1 { "y" } else { "ies" }
            ));
            return false;
        };

        log_info(&format!(
            "ServerResources: entry matched, url={} gameVersion={game_version}",
            selected.url
        ));
        start_download(selected);
    }
    // 本功能只读取响应，不改写，因此始终返回 false
    false
}

/// 找到下发的资源字段
///
/// 根级优先；为兼容把字段嵌进 `gameSetting` 的服务器实现，再往里找一层。
/// 字段名 `_appleChuResources` 与 AquaMai 的 `_aquaMaiResources` 结构一致。
fn locate_resources(response: &JsonValue) -> Option<&JsonValue> {
    response
        .get(RESOURCES_FIELD)
        .or_else(|| response.get("gameSetting")?.get(RESOURCES_FIELD))
}

/// 下发字段名（与 AquaMai 的 `_aquaMaiResources` 同构，仅换品牌前缀）
pub(crate) const RESOURCES_FIELD: &str = "_appleChuResources";

/// 从同一响应派生游戏版本整数（`romVersion` / `dataVersion`）
///
/// 取 `major*10000 + minor*100 + release`；缺失或异常时返回 0（不限制）。
fn game_version_from(response: &JsonValue) -> i64 {
    let setting = response.get("gameSetting");
    let rom_version = setting
        .and_then(|value| value.get("romVersion"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    version_to_int(rom_version)
}

fn version_to_int(text: &str) -> i64 {
    let mut parts = text.trim().split('.');
    let major = parts
        .next()
        .and_then(|part| part.trim().parse::<i64>().ok());
    let minor = parts
        .next()
        .and_then(|part| part.trim().parse::<i64>().ok());
    let release = parts
        .next()
        .and_then(|part| part.trim().parse::<i64>().ok());
    match (major, minor, release) {
        (Some(major), Some(minor), Some(release)) => major * 10_000 + minor * 100 + release,
        (Some(major), Some(minor), None) => major * 10_000 + minor * 100,
        _ => 0,
    }
}

/// 启动后台下载（只允许一次）
///
/// 下载与解压在独立线程，避免阻塞收包线程；期间由开机闸门把
/// `projGame::PowerOn` 状态机停在当前帧，结束后放行。
fn start_download(selected: &entry::ServerResourcesEntry) {
    if DOWNLOAD_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    power_on_gate::begin_block();
    let url = selected.url.clone();
    let sign = selected.sign.clone();
    std::thread::spawn(move || {
        if let Err(error) = fetch_and_mount(&url, &sign) {
            log_warn(&format!("ServerResources: {error}"));
        }
        // 无论成功失败都恢复开机，避免下载异常时卡住
        power_on_gate::end_block();
    });
}

fn fetch_and_mount(url: &str, sign: &str) -> Result<(), String> {
    let Some(api) = crate::util::api::API.get() else {
        return Err("api handle is not available".to_owned());
    };
    let Some((public_key, cache_path)) = api_config() else {
        return Err("ServerResources config is not available".to_owned());
    };

    let cache = resolve_cache_path(&cache_path);

    // 1) 缓存命中：签名一致则直接加载，不重新下载
    if let Ok(cached) = std::fs::read(&cache) {
        if crate::util::crypto::verify_signature(&public_key, &cached, sign) {
            api.log_info("ServerResources: cache hit, loading from cache");
            return mount_package(&cached);
        }
        api.log_warn("ServerResources: cache signature mismatch, re-downloading");
        let _ = std::fs::remove_file(&cache);
    }

    // 2) 下载 → 验签 → 写缓存
    let bytes = download::download(url)?;
    if !crate::util::crypto::verify_signature(&public_key, &bytes, sign) {
        return Err("downloaded package failed signature verification".to_owned());
    }
    if let Some(parent) = std::path::Path::new(&cache).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&cache, &bytes) {
        api.log_warn(&format!("ServerResources: cache write failed: {error}"));
    }
    mount_package(&bytes)
}

/// 解析包并挂载：解压到缓存目录，再把 option 子树重定向过去
fn mount_package(bytes: &[u8]) -> Result<(), String> {
    let package = package::OptionPackage::parse(bytes)?;
    let root = package.root().to_owned();
    let directory = extract_dir(&root)?;

    // 覆盖安装：先清掉上一次的内容，避免残留已从包中移除的文件
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let written = package.extract_to(&directory)?;

    // 挂载：`c:\mount\option\<root>\...` 指向解压目录
    let target = directory.to_string_lossy().into_owned();
    vfs::set_option_override(&root, &target);
    if let Ok(mut mounted) = mounted().lock() {
        mounted.retain(|entry| entry.root != root);
        mounted.push(MountedOption {
            root: root.clone(),
            directory: directory.clone(),
        });
    }
    log_info(&format!(
        "ServerResources: option mounted root={root} files={written} dir={}",
        directory.display()
    ));
    Ok(())
}

/// 缓存文件绝对路径（相对游戏目录）
fn resolve_cache_path(configured: &str) -> String {
    let base = crate::proxy::base_dir().unwrap_or_else(|| ".".to_owned());
    let relative = configured.trim();
    if relative.is_empty() {
        format!("{base}\\tmp\\ServerResources")
    } else {
        let normalized = relative.replace('/', "\\");
        if normalized.contains(':') {
            normalized
        } else {
            format!("{base}\\{normalized}")
        }
    }
}

/// 某个自定义 option 的解压目录：缓存路径加 `.d` 后缀
///
/// 解压目录与缓存文件同名不同层级，改 `cache_path` 时两者一起走。
fn extract_dir(root: &str) -> Result<PathBuf, String> {
    let Some((_, cache_path)) = api_config() else {
        return Err("ServerResources config is not available".to_owned());
    };
    let cache = resolve_cache_path(&cache_path);
    let mut directory = PathBuf::from(format!("{cache}.d"));
    // 包内根目录名已由 `package` 校验过（无分隔符与 `..`），这里再挡一次
    if root.is_empty() || root.contains(['\\', '/', ':']) || root.contains("..") {
        return Err(format!("option package has an unsafe root name: {root}"));
    }
    directory.push(root);
    Ok(directory)
}

/// 取当前配置中的 ServerResources 段（公钥与缓存路径）
fn api_config() -> Option<(String, String)> {
    let base = crate::proxy::base_dir()?;
    let config = Config::global(&base);
    config
        .section::<ServerResourcesConfig>()
        .map(|section| (section.public_key.clone(), section.cache_path.clone()))
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
    fn game_version_builds_from_semver_text() {
        // Given: 服务器下发的 romVersion 文本。
        // Then: 转成 major*10000 + minor*100 + release。
        assert_eq!(version_to_int("2.50.00"), 25000);
        assert_eq!(version_to_int("2.50.01"), 25001);
        assert_eq!(version_to_int("2.45"), 24500);
    }

    #[test]
    fn malformed_version_is_unrestricted() {
        // Given: 非数字版本串。
        // Then: 归零，条件筛选视为不限制。
        assert_eq!(version_to_int(""), 0);
        assert_eq!(version_to_int("abc"), 0);
        assert_eq!(version_to_int("2.x.1"), 0);
    }

    #[test]
    fn game_version_reads_from_response() {
        // Given: 含 gameSetting.romVersion 的响应。
        let value = json::parse(r#"{"gameSetting":{"romVersion":"2.50.00"}}"#).unwrap();

        // Then: 版本整数被正确派生。
        assert_eq!(game_version_from(&value), 25000);
        assert_eq!(game_version_from(&json::parse("{}").unwrap()), 0);
    }

    #[test]
    fn cache_path_is_relative_to_game_directory() {
        // Given: 配置中的相对缓存路径。
        // Then: 展开到游戏目录下，且不因缺少 base_dir 而 panic。
        let path = resolve_cache_path("tmp/ServerResources");
        assert!(path.ends_with("tmp\\ServerResources"));
    }

    #[test]
    fn extract_directory_sits_next_to_the_cache_file() {
        // Given: 一个合法的包内根目录名。
        // Then: 解压目录是缓存路径加 `.d`，再拼上根目录名。
        let directory = extract_dir("A123").expect("应能构造解压目录");
        assert!(directory
            .to_string_lossy()
            .ends_with("ServerResources.d\\A123"));
    }

    #[test]
    fn unsafe_root_names_are_rejected() {
        // Given: 含分隔符或穿越段的根目录名。
        // Then: 一律拒绝，避免解压写出缓存目录之外。
        assert!(extract_dir("").is_err());
        assert!(extract_dir("..").is_err());
        assert!(extract_dir("a\\b").is_err());
        assert!(extract_dir("C:").is_err());
    }
}
