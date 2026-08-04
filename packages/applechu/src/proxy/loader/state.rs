use std::ffi::c_void;
use std::fs::File;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::HANDLE;

use chu_abi::{ChuModFrameFunc, ChuModReadyFunc, ChuModShutdownFunc};

pub type HMODULE = *mut c_void;

#[derive(Clone, Copy)]
pub enum OutputSink {
    None,
    Console { handle: HANDLE, ansi_enabled: bool },
    Stream(HANDLE),
}

pub struct LoadedMod {
    pub handle: HMODULE,
    pub on_ready: Option<ChuModReadyFunc>,
    pub on_frame: Option<ChuModFrameFunc>,
    pub shutdown: Option<ChuModShutdownFunc>,
    pub file_name: String,
    pub full_path: String,
    pub name: String,
}

unsafe impl Send for LoadedMod {}

pub struct LoaderState {
    pub loaded: bool,
    pub builtin_loaded: bool,
    pub mods: Vec<LoadedMod>,
    pub base_dir: String,
    pub manifest_paths: Vec<String>,
    pub log_file: Option<File>,
    pub current_mod_log_file: Option<File>,
    pub output: OutputSink,
}

unsafe impl Send for LoaderState {}

impl Default for LoaderState {
    fn default() -> Self {
        Self {
            loaded: false,
            builtin_loaded: false,
            mods: Vec::new(),
            base_dir: String::new(),
            manifest_paths: Vec::new(),
            log_file: None,
            current_mod_log_file: None,
            output: OutputSink::None,
        }
    }
}

pub static STATE: once_cell::sync::Lazy<Mutex<LoaderState>> =
    once_cell::sync::Lazy::new(|| Mutex::new(LoaderState::default()));
