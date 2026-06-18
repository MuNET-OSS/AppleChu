use crate::config::Config;
use crate::platform::winapi::{
    self, FileTime, GetLocalTimeFn, GetSystemTimeAsFileTimeFn, GetSystemTimeFn,
    GetTimeZoneInformationFn, SetLocalTimeFn, SetSystemTimeFn, SetTimeZoneInformationFn,
    SystemTime, TimeZoneInformation,
};
use crate::util::api::Api;

static mut ORIG_GET_SYSTEM_TIME_AS_FILE_TIME: Option<GetSystemTimeAsFileTimeFn> = None;
static mut ORIG_GET_LOCAL_TIME: Option<GetLocalTimeFn> = None;
static mut ORIG_GET_SYSTEM_TIME: Option<GetSystemTimeFn> = None;
static mut ORIG_GET_TIME_ZONE_INFORMATION: Option<GetTimeZoneInformationFn> = None;
static mut ORIG_SET_LOCAL_TIME: Option<SetLocalTimeFn> = None;
static mut ORIG_SET_SYSTEM_TIME: Option<SetSystemTimeFn> = None;
static mut ORIG_SET_TIME_ZONE_INFORMATION: Option<SetTimeZoneInformationFn> = None;
static mut CONFIG: ClockConfig = ClockConfig {
    timezone: TimezoneMode::Real,
    timewarp_seconds: 0,
    writeable: false,
};

#[derive(Clone, Copy)]
struct ClockConfig {
    timezone: TimezoneMode,
    timewarp_seconds: i64,
    writeable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimezoneMode {
    Real,
    Jst,
}

pub fn init(api: &Api, config: &Config) {
    if !config.get_bool("Clock", "enable", true) {
        return;
    }

    unsafe {
        CONFIG = ClockConfig {
            timezone: match config
                .get_string("Clock", "timezone", "jst")
                .to_ascii_lowercase()
                .as_str()
            {
                "real" | "local" | "system" => TimezoneMode::Real,
                _ => TimezoneMode::Jst,
            },
            timewarp_seconds: config.get_int("Clock", "timewarp", 0),
            writeable: config.get_bool("Clock", "writeable", false),
        };
        ORIG_GET_SYSTEM_TIME_AS_FILE_TIME = winapi::hook_import(
            api,
            "kernel32.dll",
            "GetSystemTimeAsFileTime",
            hooked_get_system_time_as_file_time as *const (),
        );
        ORIG_GET_LOCAL_TIME = winapi::hook_import(
            api,
            "kernel32.dll",
            "GetLocalTime",
            hooked_get_local_time as *const (),
        );
        ORIG_GET_SYSTEM_TIME = winapi::hook_import(
            api,
            "kernel32.dll",
            "GetSystemTime",
            hooked_get_system_time as *const (),
        );
        ORIG_GET_TIME_ZONE_INFORMATION = winapi::hook_import(
            api,
            "kernel32.dll",
            "GetTimeZoneInformation",
            hooked_get_time_zone_information as *const (),
        );
        ORIG_SET_LOCAL_TIME = winapi::hook_import(
            api,
            "kernel32.dll",
            "SetLocalTime",
            hooked_set_local_time as *const (),
        );
        ORIG_SET_SYSTEM_TIME = winapi::hook_import(
            api,
            "kernel32.dll",
            "SetSystemTime",
            hooked_set_system_time as *const (),
        );
        ORIG_SET_TIME_ZONE_INFORMATION = winapi::hook_import(
            api,
            "kernel32.dll",
            "SetTimeZoneInformation",
            hooked_set_time_zone_information as *const (),
        );
    }

    api.log_info("clock hook initialized");
}

pub fn shutdown() {}

unsafe extern "system" fn hooked_get_system_time_as_file_time(file_time: *mut FileTime) {
    if let Some(orig) = ORIG_GET_SYSTEM_TIME_AS_FILE_TIME {
        orig(file_time);
    }
    if !file_time.is_null() && CONFIG.timewarp_seconds != 0 {
        add_seconds(file_time, CONFIG.timewarp_seconds);
    }
}

unsafe extern "system" fn hooked_get_system_time(system_time: *mut SystemTime) {
    if let Some(orig) = ORIG_GET_SYSTEM_TIME {
        orig(system_time);
    }
}

unsafe extern "system" fn hooked_get_local_time(system_time: *mut SystemTime) {
    if let Some(orig) = ORIG_GET_LOCAL_TIME {
        orig(system_time);
    }
}

unsafe extern "system" fn hooked_get_time_zone_information(info: *mut TimeZoneInformation) -> u32 {
    if CONFIG.timezone == TimezoneMode::Jst && !info.is_null() {
        *info = jst_timezone();
        return 0;
    }

    ORIG_GET_TIME_ZONE_INFORMATION.map_or(0xFFFF_FFFF, |orig| orig(info))
}

unsafe extern "system" fn hooked_set_local_time(time: *const SystemTime) -> i32 {
    if CONFIG.writeable {
        ORIG_SET_LOCAL_TIME.map_or(0, |orig| orig(time))
    } else {
        1
    }
}

unsafe extern "system" fn hooked_set_system_time(time: *const SystemTime) -> i32 {
    if CONFIG.writeable {
        ORIG_SET_SYSTEM_TIME.map_or(0, |orig| orig(time))
    } else {
        1
    }
}

unsafe extern "system" fn hooked_set_time_zone_information(
    info: *const TimeZoneInformation,
) -> i32 {
    if CONFIG.writeable {
        ORIG_SET_TIME_ZONE_INFORMATION.map_or(0, |orig| orig(info))
    } else {
        1
    }
}

unsafe fn add_seconds(file_time: *mut FileTime, seconds: i64) {
    let value = ((*file_time).high_date_time as u64) << 32 | (*file_time).low_date_time as u64;
    let shifted = if seconds >= 0 {
        value.saturating_add(seconds as u64 * 10_000_000)
    } else {
        value.saturating_sub(seconds.unsigned_abs() * 10_000_000)
    };
    (*file_time).low_date_time = shifted as u32;
    (*file_time).high_date_time = (shifted >> 32) as u32;
}

fn jst_timezone() -> TimeZoneInformation {
    let mut info = TimeZoneInformation {
        bias: -540,
        ..TimeZoneInformation::default()
    };
    let name = "Tokyo Standard Time".encode_utf16().collect::<Vec<_>>();
    for (index, unit) in name.into_iter().take(31).enumerate() {
        info.standard_name[index] = unit;
    }
    info
}
