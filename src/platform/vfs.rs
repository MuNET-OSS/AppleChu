use std::ffi::CString;
use std::path::PathBuf;
use std::sync::OnceLock;

use once_cell::sync::OnceCell;

use crate::config::Config;
use crate::iohook::proc_addr;
use crate::platform::reg_hook::{self, HKEY_LOCAL_MACHINE, RegValue};
use crate::platform::{path_hook, winapi};
use crate::util::api::Api;

static CONFIG: OnceCell<VfsConfig> = OnceCell::new();
static OPTION_PATH_WIDE: OnceLock<Vec<u16>> = OnceLock::new();

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

pub fn init(api: &Api, config: &Config) {
    if !config.get_bool("VFS", "enable", true) {
        return;
    }

    let amfs = winapi::fixup_path(&winapi::absolutize(
        &config.base_dir,
        &config.get_string("VFS", "amfs", "amfs"),
    ));
    let appdata = winapi::fixup_path(&winapi::absolutize(
        &config.base_dir,
        &config.get_string("VFS", "appdata", "appdata"),
    ));
    let option = winapi::fixup_path(&winapi::absolutize(
        &config.base_dir,
        &config.get_string("VFS", "option", "option"),
    ));
    let nthome = winapi::fixup_path(std::path::Path::new(&winapi::userprofile()));

    winapi::mkdir_rec(&amfs);
    winapi::mkdir_rec(&appdata);
    winapi::mkdir_rec(&format!("{nthome}temp"));

    let _ = CONFIG.set(VfsConfig {
        amfs,
        option: option.clone(),
        appdata,
        nthome,
        allow_amfs_downloads: config.get_bool("VFS", "allowAmfsDownloads", false),
    });
    path_hook::push(vfs_path_transform);
    push_registry_key();

    let wide: Vec<u16> = option.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = OPTION_PATH_WIDE.set(wide);
    proc_addr::push_get_proc_override("amdaemon_api.dll", option_proc_override);

    api.log_info("VFS hook initialized");
}

pub fn shutdown() {}

fn option_proc_override(_module: usize, name: &str) -> Option<*const ()> {
    if name == "AppImage_getOptionMountRootPath" {
        return Some(hooked_get_option_mount_root_path as *const ());
    }
    None
}

unsafe extern "system" fn hooked_get_option_mount_root_path() -> *const u16 {
    OPTION_PATH_WIDE
        .get()
        .map_or(std::ptr::null(), |path| path.as_ptr())
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
    resolve_path(path).map(|path| path.to_string_lossy().into_owned())
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
