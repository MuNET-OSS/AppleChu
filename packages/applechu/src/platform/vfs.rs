use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::OnceCell;

use crate::config::Config;
use crate::iohook::proc_addr;
use crate::platform::reg_hook::{self, RegValue, HKEY_LOCAL_MACHINE};
use crate::platform::{path_hook, winapi};
use crate::util::api::Api;

static CONFIG: OnceCell<VfsConfig> = OnceCell::new();
static OPTION_API_LOGGED: AtomicBool = AtomicBool::new(false);
static OPTION_PATH_LOGGED: AtomicBool = AtomicBool::new(false);

const VFS_NTHOME: &str = "c:\\documents and settings\\appuser";
const VFS_W10HOME: &str = "c:\\users\\appuser";
const VFS_OPTION: &str = "c:\\mount\\option";
const VFS_APM: &str = "c:\\mount\\apm";
const VFS_TMP_ICF: &str = "e:\\tmpicf.icf";

#[derive(Clone)]
struct VfsConfig {
    amfs: String,
    option: String,
    appdata: String,
    nthome: String,
    allow_amfs_downloads: bool,
}

crate::config_section! {
    pub(crate) struct VfsSectionConfig => VFS_CONFIG_SECTION {
        section: "VFS",
        order: 970,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "虚拟文件系统",
        fields: {
            pub amfs: String = String::from("amfs"),
            comment: "AMFS 目录";
            pub appdata: String = String::from("appdata"),
            comment: "APPDATA 目录";
            pub option: String = String::from("../option"),
            comment: "选项资源目录";
            pub allow_amfs_downloads: bool = false,
            comment: "允许写入 AMFS 下载内容";
        }
    }
}

#[applechu_macros::config_section(stage = Platform, order = 90)]
pub(crate) fn init(api: &Api, config: &Config, section: &VfsSectionConfig) {
    let amfs = winapi::fixup_path(&winapi::absolutize(config.base_dir(), &section.amfs));
    let appdata = winapi::fixup_path(&winapi::absolutize(config.base_dir(), &section.appdata));
    let configured_option = winapi::absolutize(config.base_dir(), &section.option);
    let option = winapi::fixup_path(&select_option_path(config.base_dir(), &configured_option));
    let nthome = winapi::fixup_path(std::path::Path::new(&winapi::userprofile()));

    winapi::mkdir_rec(&amfs);
    winapi::mkdir_rec(&appdata);
    winapi::mkdir_rec(&format!("{nthome}temp"));

    let _ = CONFIG.set(VfsConfig {
        amfs,
        option,
        appdata,
        nthome,
        allow_amfs_downloads: section.allow_amfs_downloads,
    });
    path_hook::push(vfs_path_transform);
    push_registry_key();

    proc_addr::push_get_proc_override("amdaemon_api.dll", option_proc_override);

    if let Some(config) = CONFIG.get() {
        api.log_info(&format!(
            "Virtual file system ready: option={}",
            config.option
        ));
    }
}

fn option_proc_override(_module: usize, name: &str) -> Option<*const ()> {
    match name {
        "System_getAppRootPath" => Some(hooked_get_app_root_path as *const ()),
        "AppImage_getOptionMountRootPath" => Some(hooked_get_option_mount_root_path as *const ()),
        _ => None,
    }
}

unsafe extern "system" fn hooked_get_app_root_path() -> *mut u16 {
    let Some(config) = CONFIG.get() else {
        return std::ptr::null_mut();
    };
    owned_wide_path(&format!("{}SDHD\\", config.appdata))
}

unsafe extern "system" fn hooked_get_option_mount_root_path() -> *mut u16 {
    CONFIG.get().map_or(std::ptr::null_mut(), |config| {
        if !OPTION_API_LOGGED.swap(true, Ordering::AcqRel) {
            log_info(&format!("Option mount requested: {}", config.option));
        }
        owned_wide_path(&config.option)
    })
}

fn owned_wide_path(path: &str) -> *mut u16 {
    // amdaemon_api 要求返回可写路径指针，因此让每次响应存活到进程结束，
    // 避免把 Rust 临时缓冲区交给原生代码
    Box::leak(
        path.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
    .as_mut_ptr()
}

pub fn resolve_path(path: &str) -> Option<PathBuf> {
    let config = CONFIG.get()?;
    let normalized = winapi::normalize_path(path);

    if normalized == VFS_TMP_ICF && !config.allow_amfs_downloads {
        return Some(PathBuf::from(&config.appdata).join("tmpIcf.icf"));
    }
    if let Some(tail) = strip_drive(&normalized, 'e') {
        return Some(join_root(&config.amfs, tail));
    }
    if let Some(tail) = strip_drive(&normalized, 'y') {
        return Some(join_root(&config.appdata, tail));
    }
    if let Some(tail) = strip_prefix_dir(&normalized, VFS_NTHOME) {
        return Some(join_root(&config.nthome, tail));
    }
    if let Some(tail) = strip_prefix_dir(&normalized, VFS_W10HOME) {
        return Some(join_root(&config.nthome, tail));
    }
    if let Some(tail) = strip_prefix_dir(&normalized, VFS_OPTION) {
        return Some(join_root(&config.option, tail));
    }
    if let Some(tail) = strip_prefix_dir(&normalized, VFS_APM) {
        return Some(join_root(&config.option, tail));
    }
    None
}

fn strip_drive(path: &str, drive: char) -> Option<&str> {
    let bytes = path.as_bytes();
    if bytes.len() < 2 || bytes[0] != drive as u8 || bytes[1] != b':' {
        return None;
    }
    Some(path.get(3..).unwrap_or(""))
}

fn strip_prefix_dir<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = path.strip_prefix(prefix)?;
    match rest.chars().next() {
        None => Some(""),
        Some('\\') => Some(&rest[1..]),
        Some(_) => None,
    }
}

fn join_root(root: &str, tail: &str) -> PathBuf {
    let base = PathBuf::from(root);
    if tail.is_empty() {
        base
    } else {
        winapi::clean_join(&base, tail)
    }
}

fn vfs_path_transform(path: &str) -> Option<String> {
    let resolved = resolve_path(path)?.to_string_lossy().into_owned();
    let normalized = winapi::normalize_path(path);
    if (normalized == VFS_OPTION
        || normalized.starts_with(&format!("{VFS_OPTION}\\"))
        || normalized == VFS_APM
        || normalized.starts_with(&format!("{VFS_APM}\\")))
        && !OPTION_PATH_LOGGED.swap(true, Ordering::AcqRel)
    {
        log_info(&format!("Option path redirected: {path} -> {resolved}"));
    }
    Some(resolved)
}

fn log_info(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}

pub fn root_cstring(kind: &str) -> Option<CString> {
    let config = CONFIG.get()?;
    let path = match kind {
        "amfs" => &config.amfs,
        "option" => &config.option,
        "appdata" => &config.appdata,
        _ => return None,
    };
    Some(winapi::to_cstring_lossy(path))
}

pub fn amfs_path() -> Option<String> {
    CONFIG.get().map(|config| config.amfs.clone())
}

pub fn appdata_path() -> Option<String> {
    CONFIG.get().map(|config| config.appdata.clone())
}

fn push_registry_key() {
    reg_hook::push_key(
        HKEY_LOCAL_MACHINE,
        "SYSTEM\\SEGA\\SystemProperty\\mount",
        vec![
            RegValue::string("AMFS", "E:\\"),
            RegValue::string("APPDATA", "Y:\\"),
        ],
    );
}

fn select_option_path(base_dir: &Path, configured: &Path) -> PathBuf {
    if is_non_empty_directory(configured) {
        return configured.to_path_buf();
    }

    let parent_dir = base_dir.parent().unwrap_or(base_dir);
    let candidates = [
        base_dir.join("option"),
        parent_dir.join("option"),
        base_dir.join("options"),
        parent_dir.join("options"),
    ];
    candidates
        .into_iter()
        .find(|candidate| is_non_empty_directory(candidate))
        .unwrap_or_else(|| configured.to_path_buf())
}

fn is_non_empty_directory(path: &Path) -> bool {
    let Ok(mut entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.next().is_some_and(|entry| entry.is_ok())
}

#[cfg(test)]
mod tests;
