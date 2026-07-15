use std::ffi::CStr;

use windows_sys::Win32::System::LibraryLoader::GetProcAddress;

use chu_abi::{ChuModStringFunc, LOADER_VERSION};

use super::log::log_info;
use super::state::HMODULE;

#[derive(Default)]
pub struct ModMetadata {
    pub version: Option<String>,
    pub min_loader_version: Option<String>,
}

unsafe fn read_string_export(handle: HMODULE, export: &'static [u8]) -> Option<String> {
    let proc = GetProcAddress(handle, export.as_ptr())?;
    let func: ChuModStringFunc = std::mem::transmute(proc);
    let raw = func();
    if raw.is_null() {
        return None;
    }
    Some(CStr::from_ptr(raw).to_string_lossy().into_owned())
}

pub unsafe fn read_metadata(handle: HMODULE) -> ModMetadata {
    ModMetadata {
        version: read_string_export(handle, b"chumod_version\0"),
        min_loader_version: read_string_export(handle, b"chumod_min_loader_version\0")
            .filter(|v| !v.trim().is_empty()),
    }
}

fn parse_semver(version: &str) -> [u32; 3] {
    let mut parsed = [0u32; 3];
    for (index, part) in version.split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        parsed[index] = digits.parse().unwrap_or(0);
    }
    parsed
}

fn is_loader_version_satisfied(min_loader_version: &str) -> bool {
    parse_semver(LOADER_VERSION) >= parse_semver(min_loader_version)
}

pub fn should_load_metadata(mod_name: &str, metadata: &ModMetadata) -> bool {
    if let Some(min_loader_version) = &metadata.min_loader_version {
        if !is_loader_version_satisfied(min_loader_version) {
            log_info(&format!(
                "mod requires loader >= {}, current {}, skip: {}",
                min_loader_version, LOADER_VERSION, mod_name
            ));
            return false;
        }
    }
    true
}
