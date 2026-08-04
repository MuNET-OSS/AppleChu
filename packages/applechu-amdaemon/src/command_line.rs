use std::arch::asm;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

static COMMAND_LINE_A: AtomicPtr<c_char> = AtomicPtr::new(std::ptr::null_mut());
static COMMAND_LINE_W: AtomicPtr<u16> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_WGETMAINARGS: AtomicUsize = AtomicUsize::new(0);

#[link(name = "kernel32")]
extern "system" {
    fn GetCommandLineW() -> *mut u16;
    fn GetModuleHandleW(module_name: *const u16) -> usize;
    fn GetProcAddress(module: usize, proc_name: *const u8) -> *const ();
}

type AnsiCommandLineSlot = unsafe extern "C" fn() -> *mut *mut c_char;
type WideCommandLineSlot = unsafe extern "C" fn() -> *mut *mut u16;
type WgetmainargsFn =
    unsafe extern "C" fn(*mut i32, *mut *mut *mut u16, *mut *mut *mut u16, i32, *mut c_void) -> i32;

pub(crate) fn prepare(base_dir: &str) {
    if !winhttp::amdaemon::append_config_args(base_dir) {
        return;
    }
    let config_files = winhttp::amdaemon::config_files(base_dir);
    let Some(current) = current_command_line() else {
        crate::console::warn("Unable to read the AM Daemon command line");
        return;
    };
    if has_complete_config_args(&current, &config_files) {
        return;
    }
    let mut replacement = strip_config_args(&current);
    replacement.push_str(" -c");
    // AM Daemon 按固定顺序从工作目录解析这些配置文件
    for file in &config_files {
        replacement.push(' ');
        replacement.push_str(file);
    }

    if let Err(error) = unsafe { replace_process_command_line(&replacement) } {
        crate::console::error(&format!(
            "Unable to append AM Daemon config arguments: {error}"
        ));
    } else {
        crate::console::info(&format!(
            "Prepared {} AM Daemon config files",
            config_files.len()
        ));
    }
}

fn current_command_line() -> Option<String> {
    unsafe {
        let ptr = GetCommandLineW();
        if ptr.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(
            ptr, len,
        )))
    }
}

fn has_complete_config_args(command_line: &str, config_files: &[String]) -> bool {
    let arguments = split_command_line(command_line);
    let Some(index) = arguments
        .iter()
        .position(|argument| argument.eq_ignore_ascii_case("-c"))
    else {
        return false;
    };
    let configs = &arguments[index + 1..];
    if configs.len() != config_files.len() {
        return false;
    }
    configs.iter().zip(config_files).all(|(actual, expected)| {
        std::path::Path::new(actual)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(expected))
    })
}

fn strip_config_args(command_line: &str) -> String {
    let Some((start, _)) = argument_spans(command_line)
        .into_iter()
        .find(|(_, argument)| argument.eq_ignore_ascii_case("-c"))
    else {
        return command_line.trim_end().to_owned();
    };
    command_line[..start].trim_end().to_owned()
}

fn split_command_line(command_line: &str) -> Vec<String> {
    argument_spans(command_line)
        .into_iter()
        .map(|(_, argument)| argument)
        .collect()
}

fn argument_spans(command_line: &str) -> Vec<(usize, String)> {
    let mut arguments = Vec::new();
    let mut chars = command_line.char_indices().peekable();
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        let start = chars.peek().map_or(0, |(index, _)| *index);
        let mut argument = String::new();
        let mut quoted = false;
        while let Some((_, ch)) = chars.peek().copied() {
            if ch == '"' {
                quoted = !quoted;
                chars.next();
            } else if ch.is_whitespace() && !quoted {
                break;
            } else {
                argument.push(ch);
                chars.next();
            }
        }
        arguments.push((start, argument));
    }
    arguments
}

unsafe fn replace_process_command_line(command_line: &str) -> Result<usize, &'static str> {
    let mut buffer = command_line.encode_utf16().collect::<Vec<_>>();
    let byte_len = buffer
        .len()
        .checked_mul(2)
        .and_then(|length| u16::try_from(length).ok())
        .ok_or("replacement command line is too long")?;
    buffer.push(0);
    let maximum_length = buffer
        .len()
        .checked_mul(2)
        .and_then(|length| u16::try_from(length).ok())
        .ok_or("replacement command line is too long")?;
    let buffer = Box::leak(buffer.into_boxed_slice());
    COMMAND_LINE_W.store(buffer.as_mut_ptr(), Ordering::Release);

    let mut ansi = command_line.as_bytes().to_vec();
    if ansi.contains(&0) {
        return Err("replacement command line contains a nul byte");
    }
    ansi.push(0);
    let ansi = Box::leak(ansi.into_boxed_slice());
    COMMAND_LINE_A.store(ansi.as_mut_ptr().cast(), Ordering::Release);

    let peb: usize;
    asm!("mov {}, gs:[0x60]", out(reg) peb, options(nostack, preserves_flags));
    if peb == 0 {
        return Err("PEB is unavailable");
    }
    let process_parameters = *(peb.checked_add(0x20).ok_or("invalid PEB")? as *const usize);
    if process_parameters == 0 {
        return Err("process parameters are unavailable");
    }
    let command_line = (process_parameters + 0x70) as *mut UnicodeString;
    (*command_line).buffer = buffer.as_mut_ptr();
    (*command_line).length = byte_len;
    (*command_line).maximum_length = maximum_length;

    let imports = winhttp::amdaemon::install_command_line_hooks(
        hooked_get_command_line_a as *const (),
        hooked_get_command_line_w as *const (),
    );
    install_wgetmainargs_probe();
    Ok(imports + patch_msvcr_command_line())
}

unsafe fn install_wgetmainargs_probe() {
    let mut original = std::ptr::null();
    let _patched = winhttp::amdaemon::install_wgetmainargs_hook(
        hooked_wgetmainargs as *const (),
        &mut original,
    );
    if !original.is_null() {
        ORIGINAL_WGETMAINARGS.store(original as usize, Ordering::Release);
    }
}

unsafe extern "C" fn hooked_wgetmainargs(
    argc: *mut i32,
    argv: *mut *mut *mut u16,
    env: *mut *mut *mut u16,
    expand_wildcards: i32,
    startup_info: *mut c_void,
) -> i32 {
    let original = ORIGINAL_WGETMAINARGS.load(Ordering::Acquire);
    if original == 0 {
        crate::console::error("AM Daemon argument parser is unavailable");
        return -1;
    }

    let original: WgetmainargsFn = std::mem::transmute(original);
    let result = original(argc, argv, env, expand_wildcards, startup_info);
    validate_wargv(result, argc, argv);
    result
}

unsafe fn validate_wargv(result: i32, argc: *const i32, argv: *const *mut *mut u16) {
    if result != 0 || argc.is_null() || argv.is_null() || (*argv).is_null() {
        crate::console::warn(&format!(
            "AM Daemon argument parsing failed: result={result}"
        ));
    }
}

unsafe extern "system" fn hooked_get_command_line_a() -> *mut c_char {
    COMMAND_LINE_A.load(Ordering::Acquire)
}

unsafe extern "system" fn hooked_get_command_line_w() -> *mut u16 {
    COMMAND_LINE_W.load(Ordering::Acquire)
}

unsafe fn patch_msvcr_command_line() -> usize {
    let module_name = "msvcr110.dll\0".encode_utf16().collect::<Vec<_>>();
    let module = GetModuleHandleW(module_name.as_ptr());
    if module == 0 {
        return 0;
    }

    let mut patched = 0;
    let ansi_proc = GetProcAddress(module, b"__p__acmdln\0".as_ptr());
    if !ansi_proc.is_null() {
        let get_slot: AnsiCommandLineSlot = std::mem::transmute(ansi_proc);
        let slot = get_slot();
        if !slot.is_null() {
            *slot = COMMAND_LINE_A.load(Ordering::Acquire);
            patched += 1;
        }
    }

    let wide_proc = GetProcAddress(module, b"__p__wcmdln\0".as_ptr());
    if !wide_proc.is_null() {
        let get_slot: WideCommandLineSlot = std::mem::transmute(wide_proc);
        let slot = get_slot();
        if !slot.is_null() {
            *slot = COMMAND_LINE_W.load(Ordering::Acquire);
            patched += 1;
        }
    }
    patched
}

#[cfg(test)]
mod tests {
    use super::{has_complete_config_args, split_command_line, strip_config_args};

    fn config_files() -> Vec<String> {
        [
            "config_common.json",
            "config_server.json",
            "config_client.json",
            "config_cvt.json",
            "config_sp.json",
            "config_hook.json",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[test]
    fn bare_config_switch_is_not_complete() {
        assert!(!has_complete_config_args(
            "amdaemon.exe -c",
            &config_files()
        ));
        assert_eq!(strip_config_args("amdaemon.exe -c"), "amdaemon.exe");
    }

    #[test]
    fn default_config_list_is_complete() {
        assert!(has_complete_config_args(
            "amdaemon.exe -c config_common.json config_server.json config_client.json config_cvt.json config_sp.json config_hook.json",
            &config_files(),
        ));
    }

    #[test]
    fn quoted_paths_are_parsed_as_single_arguments() {
        let arguments = split_command_line(
            "\"D:\\Game Dir\\amdaemon.exe\" -c \"D:\\Game Dir\\config_common.json\"",
        );
        assert_eq!(arguments[0], "D:\\Game Dir\\amdaemon.exe");
        assert_eq!(arguments[2], "D:\\Game Dir\\config_common.json");
    }
}
