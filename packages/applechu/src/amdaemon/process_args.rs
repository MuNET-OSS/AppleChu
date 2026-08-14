use super::launch::INHERIT_CONSOLE_ENV;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub(super) fn wide_path(path: &std::path::Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(super) fn wide_command_line(executable: &std::path::Path, config_files: &[String]) -> Vec<u16> {
    let mut arguments = vec![
        quote_windows_arg(&executable.to_string_lossy()),
        "-c".to_owned(),
    ];
    arguments.extend(config_files.iter().map(|file| quote_windows_arg(file)));
    OsStr::new(&arguments.join(" "))
        .encode_wide()
        .chain(Some(0))
        .collect()
}

fn quote_windows_arg(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }
    let mut quoted = String::from("\"");
    let mut slashes = 0;
    for character in value.chars() {
        match character {
            '\\' => slashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
                quoted.push('"');
                slashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', slashes));
                quoted.push(character);
                slashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', slashes * 2));
    quoted.push('"');
    quoted
}

pub(super) fn wide_environment() -> Vec<u16> {
    let mut entries = std::env::vars_os()
        .filter(|(key, _)| {
            !key.to_string_lossy()
                .eq_ignore_ascii_case(INHERIT_CONSOLE_ENV)
        })
        .map(|(key, value)| {
            let mut entry = key;
            entry.push("=");
            entry.push(value);
            entry
        })
        .collect::<Vec<_>>();
    let mut inherit_console = OsStr::new(INHERIT_CONSOLE_ENV).to_os_string();
    inherit_console.push("=1");
    entries.push(inherit_console);
    let mut environment = Vec::new();
    for entry in entries {
        environment.extend(entry.encode_wide());
        environment.push(0);
    }
    environment.push(0);
    environment
}
