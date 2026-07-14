use windows_sys_loader::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys_loader::Win32::System::Console::{
    AllocConsole, GetConsoleMode, GetStdHandle, SetConsoleMode, SetConsoleOutputCP,
    SetConsoleTitleA, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_OUTPUT_HANDLE,
};

use super::log::{write_banner_line, ANSI_CYAN};
use super::state::{LoaderState, OutputSink};
use super::{hash, pe};

const CP_UTF8: u32 = 65001;
const ANSI_GRAY: &str = "\x1b[37m";

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
    handle: windows_sys_loader::Win32::Foundation::HANDLE,
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

fn os_version() -> String {
    #[repr(C)]
    struct OsVersionInfoW {
        dw_osversion_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }

    unsafe {
        let mut info: OsVersionInfoW = std::mem::zeroed();
        info.dw_osversion_info_size = std::mem::size_of::<OsVersionInfoW>() as u32;
        if RtlGetVersion(&mut info) != 0 {
            return "Windows (unknown)".to_string();
        }
        let name = if info.dw_major_version == 10 && info.dw_build_number >= 22000 {
            "Windows 11"
        } else if info.dw_major_version == 10 {
            "Windows 10"
        } else {
            "Windows"
        };
        format!("{} (build {})", name, info.dw_build_number)
    }
}
