use std::fmt::Write;

#[repr(u8)]
#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }

    const fn ansi(self) -> &'static str {
        match self {
            Self::Info => ANSI_GRAY,
            Self::Warn => ANSI_YELLOW,
            Self::Error => ANSI_RED,
        }
    }
}

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_TIME: &str = "\x1b[92m";
const ANSI_TAG: &str = "\x1b[96m";
pub const ANSI_CYAN: &str = "\x1b[96m";
pub const ANSI_GRAY: &str = "\x1b[37m";
const ANSI_YELLOW: &str = "\x1b[93m";
const ANSI_RED: &str = "\x1b[91m";

pub fn format_lines(hour: u16, minute: u16, second: u16, level: LogLevel, message: &str) -> String {
    let mut output = String::new();
    if message.is_empty() {
        let _ = writeln!(
            output,
            "[{hour:02}:{minute:02}:{second:02}] [{}] [applechu] ",
            level.label()
        );
        return output;
    }
    for line in message.lines() {
        let _ = writeln!(
            output,
            "[{hour:02}:{minute:02}:{second:02}] [{}] [applechu] {line}",
            level.label()
        );
    }
    output
}

pub fn format_ansi_lines(
    hour: u16,
    minute: u16,
    second: u16,
    level: LogLevel,
    message: &str,
) -> String {
    format_ansi_lines_with_body(hour, minute, second, level, message, level.ansi())
}

pub fn format_ansi_lines_with_body(
    hour: u16,
    minute: u16,
    second: u16,
    level: LogLevel,
    message: &str,
    body: &str,
) -> String {
    let mut output = String::new();
    let mut write_line = |line: &str| {
        let _ = writeln!(
            output,
            "{ANSI_TIME}[{hour:02}:{minute:02}:{second:02}]{ANSI_RESET} {body}[{}]{ANSI_RESET} {ANSI_TAG}[applechu]{ANSI_RESET} {body}{line}{ANSI_RESET}",
            level.label()
        );
    };
    if message.is_empty() {
        write_line("");
    } else {
        for line in message.lines() {
            write_line(line);
        }
    }
    output
}

pub fn os_version() -> String {
    #[repr(C)]
    struct OsVersionInfoW {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfoW) -> i32;
    }

    unsafe {
        let mut info: OsVersionInfoW = std::mem::zeroed();
        info.size = std::mem::size_of::<OsVersionInfoW>() as u32;
        if RtlGetVersion(&mut info) != 0 {
            return "Windows (unknown)".to_owned();
        }
        let name = if info.major == 10 && info.build >= 22000 {
            "Windows 11"
        } else if info.major == 10 {
            "Windows 10"
        } else {
            "Windows"
        };
        format!("{name} (build {})", info.build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_has_no_milliseconds() {
        assert_eq!(
            format_lines(1, 2, 3, LogLevel::Info, "ready"),
            "[01:02:03] [INFO] [applechu] ready\n"
        );
    }

    #[test]
    fn every_message_line_has_a_prefix() {
        assert_eq!(
            format_lines(12, 34, 56, LogLevel::Warn, "first\nsecond"),
            concat!(
                "[12:34:56] [WARN] [applechu] first\n",
                "[12:34:56] [WARN] [applechu] second\n"
            )
        );
    }

    #[test]
    fn ansi_log_uses_the_shared_console_palette() {
        assert_eq!(
            format_ansi_lines(1, 2, 3, LogLevel::Warn, "ready"),
            "\x1b[92m[01:02:03]\x1b[0m \x1b[93m[WARN]\x1b[0m \x1b[96m[applechu]\x1b[0m \x1b[93mready\x1b[0m\n"
        );
    }
}
