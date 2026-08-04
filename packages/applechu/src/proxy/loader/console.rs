use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Console::{
    AllocConsole, GetConsoleMode, GetStdHandle, SetConsoleMode, SetConsoleOutputCP,
    SetConsoleTitleA, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_OUTPUT_HANDLE,
};

use super::log::{write_banner_line, ANSI_CYAN};
use super::pe;
use super::state::{LoaderState, OutputSink};
use crate::util::hash;
use crate::util::logging::{os_version, ANSI_GRAY};

const CP_UTF8: u32 = 65001;

// TODO: 控制台图标（SetConsoleIcon / 嵌入 RT_ICON 资源）
// TODO: 控制台字体大小/窗口尺寸调整

pub unsafe fn init(state: &mut LoaderState, console_enabled: bool) {
    if console_enabled {
        AllocConsole();
    }

    let handle = GetStdHandle(STD_OUTPUT_HANDLE);
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return;
    }

    if console_enabled {
        SetConsoleOutputCP(CP_UTF8);
        SetConsoleTitleA(b"AppleChu\0".as_ptr());
    }

    state.output = match console_ansi_enabled(handle, console_enabled) {
        Some(ansi_enabled) => OutputSink::Console {
            handle,
            ansi_enabled,
        },
        None => OutputSink::Stream(handle),
    };

    print_banner(state);
}

unsafe fn console_ansi_enabled(
    handle: windows_sys::Win32::Foundation::HANDLE,
    configure: bool,
) -> Option<bool> {
    let mut mode: u32 = 0;
    if GetConsoleMode(handle, &mut mode) == 0 {
        return None;
    }

    if configure {
        Some(
            SetConsoleMode(
                handle,
                mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) != 0,
        )
    } else {
        Some(mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0)
    }
}

fn print_banner(state: &mut LoaderState) {
    let version = env!("CARGO_PKG_VERSION");
    let sep = "------------------------------------------------------------";

    let hash_code = pe::get_self_path()
        .and_then(|path| hash::sha256_file(&path))
        .unwrap_or_else(|| "unknown".to_string());

    let arch = if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "x64"
    };

    write_banner_line(state, ANSI_CYAN, sep);
    write_banner_line(state, ANSI_CYAN, &format!("AppleChu v{version} Nya~ "));
    write_banner_line(state, ANSI_GRAY, &format!("OS: {}", os_version()));
    write_banner_line(state, ANSI_GRAY, &format!("Hash Code: {hash_code}"));
    write_banner_line(state, ANSI_CYAN, sep);
    write_banner_line(state, ANSI_GRAY, "Game: CHUNITHM (SDHD)");
    write_banner_line(state, ANSI_GRAY, &format!("Game Arch: {arch}"));
    write_banner_line(state, ANSI_CYAN, sep);
}
