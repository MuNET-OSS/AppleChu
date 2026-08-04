use crate::iohook::hook_table::{hook_table_apply, null_module, HookSymbol};
use crate::iohook::proc_addr;
use crate::platform::winapi::{
    FileTime, GetLocalTimeFn, GetSystemTimeAsFileTimeFn, GetTimeZoneInformationFn, SetLocalTimeFn,
    SetSystemTimeFn, SetTimeZoneInformationFn, SystemTime, TimeZoneInformation,
};
use crate::util::api::Api;
use std::ptr;
use std::sync::atomic::{AtomicI64, Ordering};

static mut ORIG_GET_SYSTEM_TIME_AS_FILE_TIME: *const () = ptr::null();
static mut ORIG_GET_LOCAL_TIME: *const () = ptr::null();
static mut ORIG_GET_SYSTEM_TIME: *const () = ptr::null();
static mut ORIG_GET_TIME_ZONE_INFORMATION: *const () = ptr::null();
static mut ORIG_SET_LOCAL_TIME: *const () = ptr::null();
static mut ORIG_SET_SYSTEM_TIME: *const () = ptr::null();
static mut ORIG_SET_TIME_ZONE_INFORMATION: *const () = ptr::null();
static mut CONFIG: ClockConfig = ClockConfig {
    timezone: TimezoneMode::Real,
    timewarp: false,
    writeable: false,
};
static CURRENT_DAY: AtomicI64 = AtomicI64::new(0);

const TICKS_PER_SECOND: i64 = 10_000_000;
const TICKS_PER_HOUR: i64 = TICKS_PER_SECOND * 3600;
const TICKS_PER_DAY: i64 = TICKS_PER_HOUR * 24;
const ERROR_INVALID_PARAMETER: u32 = 87;

#[derive(Clone, Copy)]
struct ClockConfig {
    timezone: TimezoneMode,
    timewarp: bool,
    writeable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimezoneMode {
    Real,
    Jst,
}

crate::config_section! {
    pub(crate) struct ClockSectionConfig => CLOCK_CONFIG_SECTION {
        section: "Clock",
        order: 940,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "系统时钟模拟",
        fields: {
            pub timezone: String = String::from("jst"),
            comment: "时区模式：jst 或 real";
            pub timewarp: i64 = 0,
            comment: "跳过每日维护时段；0 关闭，非 0 开启";
            pub writeable: bool = false,
            comment: "允许游戏修改系统时间";
        }
    }
}

#[applechu_macros::config_section(stage = Platform, order = 20)]
pub(crate) fn init(api: &Api, config: &ClockSectionConfig) {
    unsafe {
        CONFIG = ClockConfig {
            timezone: match config.timezone.to_ascii_lowercase().as_str() {
                "real" | "local" | "system" => TimezoneMode::Real,
                _ => TimezoneMode::Jst,
            },
            timewarp: config.timewarp != 0,
            writeable: config.writeable,
        };
        // 只安装配置实际需要的 API，并登记到 proc_addr
        // 后续动态加载的模块会自动补装这些入口
        let base_hooks = clock_base_hooks();
        let read_hooks = clock_read_hooks();
        let write_hooks = clock_write_hooks();
        let mut patched = 0;
        if CONFIG.timezone != TimezoneMode::Real || CONFIG.timewarp || !CONFIG.writeable {
            patched += hook_table_apply(null_module(), "kernel32.dll", &base_hooks);
            proc_addr::push("kernel32.dll", &base_hooks, sync_originals);
        }
        if CONFIG.timezone != TimezoneMode::Real {
            patched += hook_table_apply(null_module(), "kernel32.dll", &read_hooks);
            proc_addr::push("kernel32.dll", &read_hooks, sync_originals);
        }
        if !CONFIG.writeable {
            patched += hook_table_apply(null_module(), "kernel32.dll", &write_hooks);
            proc_addr::push("kernel32.dll", &write_hooks, sync_originals);
        }
        api.log_info(&format!(
            "Clock compatibility ready with {patched} patched entries"
        ));
    }
}

fn clock_base_hooks() -> [HookSymbol; 1] {
    [HookSymbol {
        name: "GetSystemTimeAsFileTime",
        patch: hooked_get_system_time_as_file_time as *const (),
        original: ptr::addr_of_mut!(ORIG_GET_SYSTEM_TIME_AS_FILE_TIME),
    }]
}

fn clock_read_hooks() -> [HookSymbol; 3] {
    [
        HookSymbol {
            name: "GetLocalTime",
            patch: hooked_get_local_time as *const (),
            original: ptr::addr_of_mut!(ORIG_GET_LOCAL_TIME),
        },
        HookSymbol {
            name: "GetSystemTime",
            patch: hooked_get_system_time as *const (),
            original: ptr::addr_of_mut!(ORIG_GET_SYSTEM_TIME),
        },
        HookSymbol {
            name: "GetTimeZoneInformation",
            patch: hooked_get_time_zone_information as *const (),
            original: ptr::addr_of_mut!(ORIG_GET_TIME_ZONE_INFORMATION),
        },
    ]
}

fn clock_write_hooks() -> [HookSymbol; 3] {
    [
        HookSymbol {
            name: "SetLocalTime",
            patch: hooked_set_local_time as *const (),
            original: ptr::addr_of_mut!(ORIG_SET_LOCAL_TIME),
        },
        HookSymbol {
            name: "SetSystemTime",
            patch: hooked_set_system_time as *const (),
            original: ptr::addr_of_mut!(ORIG_SET_SYSTEM_TIME),
        },
        HookSymbol {
            name: "SetTimeZoneInformation",
            patch: hooked_set_time_zone_information as *const (),
            original: ptr::addr_of_mut!(ORIG_SET_TIME_ZONE_INFORMATION),
        },
    ]
}

fn sync_originals() {}

fn clock_log(message: &str) {
    if let Some(api) = crate::util::api::API.get() {
        api.log_info(message);
    }
}

unsafe extern "system" fn hooked_get_system_time_as_file_time(file_time: *mut FileTime) {
    let original: GetSystemTimeAsFileTimeFn = match ORIG_GET_SYSTEM_TIME_AS_FILE_TIME {
        ptr if !ptr.is_null() => std::mem::transmute(ptr),
        _ => return,
    };
    if !CONFIG.timewarp {
        original(file_time);
        return;
    }
    if file_time.is_null() {
        crate::iohook::set_last_error(ERROR_INVALID_PARAMETER);
        return;
    }

    let mut real = FileTime::default();
    original(&mut real);
    let real_ticks = filetime_ticks(real);
    let biased = real_ticks + 2 * TICKS_PER_HOUR;
    let day = biased / TICKS_PER_DAY;
    let time = biased % TICKS_PER_DAY;
    let previous_day = CURRENT_DAY.swap(day, Ordering::Relaxed);
    if previous_day != 0 && previous_day != day {
        clock_log("Date changed; time warp baseline updated");
    }
    let fake = day * TICKS_PER_DAY + time * 19 / 24 - 2 * TICKS_PER_HOUR;
    *file_time = ticks_to_filetime(fake);
}

unsafe extern "system" fn hooked_get_system_time(system_time: *mut SystemTime) {
    let mut linear = FileTime::default();
    hooked_get_system_time_as_file_time(&mut linear);
    filetime_to_systemtime(&linear, system_time);
}

unsafe extern "system" fn hooked_get_local_time(system_time: *mut SystemTime) {
    if CONFIG.timezone == TimezoneMode::Real {
        let original: GetLocalTimeFn = match ORIG_GET_LOCAL_TIME {
            ptr if !ptr.is_null() => std::mem::transmute(ptr),
            _ => return,
        };
        original(system_time);
        return;
    }
    let mut linear = FileTime::default();
    hooked_get_system_time_as_file_time(&mut linear);
    linear = ticks_to_filetime(filetime_ticks(linear) + 9 * TICKS_PER_HOUR);
    filetime_to_systemtime(&linear, system_time);
}

unsafe extern "system" fn hooked_get_time_zone_information(info: *mut TimeZoneInformation) -> u32 {
    if CONFIG.timezone == TimezoneMode::Jst {
        if info.is_null() {
            crate::iohook::set_last_error(ERROR_INVALID_PARAMETER);
            return 0xFFFF_FFFF;
        }
        *info = jst_timezone();
        crate::iohook::set_last_error(0);
        clock_log("Timezone virtualized as Japan Standard Time");
        return 0;
    }

    let original: GetTimeZoneInformationFn = match ORIG_GET_TIME_ZONE_INFORMATION {
        ptr if !ptr.is_null() => std::mem::transmute(ptr),
        _ => return 0xFFFF_FFFF,
    };
    original(info)
}

unsafe extern "system" fn hooked_set_local_time(time: *const SystemTime) -> i32 {
    if CONFIG.writeable {
        let original: SetLocalTimeFn = match ORIG_SET_LOCAL_TIME {
            ptr if !ptr.is_null() => std::mem::transmute(ptr),
            _ => return 0,
        };
        original(time)
    } else {
        clock_log("Blocked local time update");
        1
    }
}

unsafe extern "system" fn hooked_set_system_time(time: *const SystemTime) -> i32 {
    if CONFIG.writeable {
        let original: SetSystemTimeFn = match ORIG_SET_SYSTEM_TIME {
            ptr if !ptr.is_null() => std::mem::transmute(ptr),
            _ => return 0,
        };
        original(time)
    } else {
        clock_log("Blocked system time update");
        1
    }
}

unsafe extern "system" fn hooked_set_time_zone_information(
    info: *const TimeZoneInformation,
) -> i32 {
    if CONFIG.writeable {
        let original: SetTimeZoneInformationFn = match ORIG_SET_TIME_ZONE_INFORMATION {
            ptr if !ptr.is_null() => std::mem::transmute(ptr),
            _ => return 0,
        };
        original(info)
    } else {
        clock_log("Blocked timezone update");
        1
    }
}

fn filetime_ticks(value: FileTime) -> i64 {
    ((i64::from(value.high_date_time)) << 32) | i64::from(value.low_date_time)
}

fn ticks_to_filetime(value: i64) -> FileTime {
    FileTime {
        low_date_time: value as u32,
        high_date_time: (value >> 32) as u32,
    }
}

unsafe fn filetime_to_systemtime(file_time: *const FileTime, system_time: *mut SystemTime) {
    if system_time.is_null() {
        crate::iohook::set_last_error(ERROR_INVALID_PARAMETER);
        return;
    }
    FileTimeToSystemTime(file_time, system_time);
}

fn jst_timezone() -> TimeZoneInformation {
    TimeZoneInformation {
        bias: -540,
        ..TimeZoneInformation::default()
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn FileTimeToSystemTime(file_time: *const FileTime, system_time: *mut SystemTime) -> i32;
}
