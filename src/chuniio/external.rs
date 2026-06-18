use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

fn string_to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

type GetApiVersionFn = unsafe extern "C" fn() -> u16;
type JvsInitFn = unsafe extern "C" fn() -> i32;
type JvsPollFn = unsafe extern "C" fn(*mut u8, *mut u8);
type JvsReadCoinFn = unsafe extern "C" fn(*mut u16);
type SliderInitFn = unsafe extern "C" fn() -> i32;
type SliderCallbackRaw = unsafe extern "C" fn(*const u8);
type SliderStartFn = unsafe extern "C" fn(SliderCallbackRaw);
type SliderStopFn = unsafe extern "C" fn();
type SliderSetLedsFn = unsafe extern "C" fn(*const u8);
type LedInitFn = unsafe extern "C" fn() -> i32;
type LedSetColorsFn = unsafe extern "C" fn(u8, *mut u8);

pub struct ExternalChuniIo {
    pub api_version: u16,
    jvs_init: JvsInitFn,
    jvs_poll: JvsPollFn,
    jvs_read_coin: JvsReadCoinFn,
    slider_init: SliderInitFn,
    slider_start: SliderStartFn,
    slider_stop: SliderStopFn,
    slider_set_leds: Option<SliderSetLedsFn>,
    led_init: Option<LedInitFn>,
    led_set_colors: Option<LedSetColorsFn>,
    jvs_started: AtomicBool,
    slider_started: AtomicBool,
    led_started: AtomicBool,
}

unsafe impl Send for ExternalChuniIo {}
unsafe impl Sync for ExternalChuniIo {}

/// Raw JVS function pointers extracted from an external IO DLL, used by the
/// chu2to3 shared-memory producer thread so it can poll the DLL directly
/// without contending for the global EXTERNAL mutex.
#[derive(Clone, Copy)]
pub struct JvsRawFns {
    pub api_version: u16,
    jvs_init: JvsInitFn,
    jvs_poll: JvsPollFn,
    jvs_read_coin: JvsReadCoinFn,
}

unsafe impl Send for JvsRawFns {}
unsafe impl Sync for JvsRawFns {}

impl JvsRawFns {
    pub unsafe fn jvs_init(&self) -> i32 {
        (self.jvs_init)()
    }

    pub unsafe fn jvs_poll(&self, opbtn: &mut u8, beams: &mut u8) {
        (self.jvs_poll)(opbtn, beams);
    }

    pub unsafe fn jvs_read_coin(&self) -> u16 {
        let mut total = 0u16;
        (self.jvs_read_coin)(&mut total);
        total
    }
}

unsafe fn resolve(module: HMODULE, name: &[u8]) -> Option<*const c_void> {
    GetProcAddress(module, name.as_ptr()).map(|proc| proc as *const c_void)
}

unsafe fn resolve_any(module: HMODULE, names: &[&[u8]]) -> Option<*const c_void> {
    names.iter().find_map(|name| resolve(module, name))
}

impl ExternalChuniIo {
    pub unsafe fn load(path: &str) -> Result<Self, String> {
        let wide = string_to_wide(path);
        let module = LoadLibraryW(wide.as_ptr());
        if module == 0 {
            return Err(format!("LoadLibraryW failed for {path}"));
        }

        let api_version = resolve_any(
            module,
            &[
                b"chuni_io_get_api_version\0",
                b"chu2to3_io_get_api_version\0",
            ],
        )
        .map(|proc| {
            let func: GetApiVersionFn = std::mem::transmute(proc);
            func()
        })
        .unwrap_or(0x0100);

        if api_version >= 0x0200 {
            return Err(format!(
                "unsupported chuniio API version {api_version:#06x}"
            ));
        }

        let jvs_init: JvsInitFn = std::mem::transmute(
            resolve_any(module, &[b"chuni_io_jvs_init\0", b"chu2to3_io_jvs_init\0"])
                .ok_or("missing chuni_io_jvs_init/chu2to3_io_jvs_init")?,
        );
        let jvs_poll: JvsPollFn = std::mem::transmute(
            resolve_any(module, &[b"chuni_io_jvs_poll\0", b"chu2to3_io_jvs_poll\0"])
                .ok_or("missing chuni_io_jvs_poll/chu2to3_io_jvs_poll")?,
        );
        let jvs_read_coin: JvsReadCoinFn = std::mem::transmute(
            resolve_any(
                module,
                &[
                    b"chuni_io_jvs_read_coin_counter\0",
                    b"chu2to3_io_jvs_read_coin_counter\0",
                ],
            )
            .ok_or("missing chuni_io_jvs_read_coin_counter/chu2to3_io_jvs_read_coin_counter")?,
        );
        let slider_init: SliderInitFn = std::mem::transmute(
            resolve_any(
                module,
                &[b"chuni_io_slider_init\0", b"chu2to3_io_slider_init\0"],
            )
            .ok_or("missing chuni_io_slider_init/chu2to3_io_slider_init")?,
        );
        let slider_start: SliderStartFn = std::mem::transmute(
            resolve_any(
                module,
                &[b"chuni_io_slider_start\0", b"chu2to3_io_slider_start\0"],
            )
            .ok_or("missing chuni_io_slider_start/chu2to3_io_slider_start")?,
        );
        let slider_stop: SliderStopFn = std::mem::transmute(
            resolve_any(
                module,
                &[b"chuni_io_slider_stop\0", b"chu2to3_io_slider_stop\0"],
            )
            .ok_or("missing chuni_io_slider_stop/chu2to3_io_slider_stop")?,
        );
        let slider_set_leds = resolve_any(
            module,
            &[
                b"chuni_io_slider_set_leds\0",
                b"chu2to3_io_slider_set_leds\0",
            ],
        )
        .map(|proc| std::mem::transmute(proc));

        let led_init = if api_version >= 0x0102 {
            resolve_any(module, &[b"chuni_io_led_init\0", b"chu2to3_io_led_init\0"])
                .map(|proc| std::mem::transmute(proc))
        } else {
            None
        };
        let led_set_colors = if api_version >= 0x0102 {
            resolve_any(
                module,
                &[b"chuni_io_led_set_colors\0", b"chu2to3_io_led_set_colors\0"],
            )
            .map(|proc| std::mem::transmute(proc))
        } else {
            None
        };

        Ok(Self {
            api_version,
            jvs_init,
            jvs_poll,
            jvs_read_coin,
            slider_init,
            slider_start,
            slider_stop,
            slider_set_leds,
            led_init,
            led_set_colors,
            jvs_started: AtomicBool::new(false),
            slider_started: AtomicBool::new(false),
            led_started: AtomicBool::new(false),
        })
    }

    unsafe fn ensure_jvs_init(&self) {
        if !self.jvs_started.swap(true, Ordering::SeqCst) {
            (self.jvs_init)();
        }
    }

    unsafe fn ensure_slider_init(&self) {
        if !self.slider_started.swap(true, Ordering::SeqCst) {
            (self.slider_init)();
        }
    }

    pub unsafe fn ensure_led_init(&self) {
        if !self.led_started.swap(true, Ordering::SeqCst) {
            if let Some(func) = self.led_init {
                func();
            }
        }
    }

    pub unsafe fn jvs_poll(&self, opbtn: &mut u8, beams: &mut u8) {
        self.ensure_jvs_init();
        (self.jvs_poll)(opbtn, beams);
    }

    /// Snapshot the raw JVS function pointers for use by the chu2to3 producer
    /// thread. The thread polls the DLL directly, matching segatools' x86
    /// jvs_poll_thread_proc behaviour.
    pub fn jvs_raw_fns(&self) -> JvsRawFns {
        JvsRawFns {
            api_version: self.api_version,
            jvs_init: self.jvs_init,
            jvs_poll: self.jvs_poll,
            jvs_read_coin: self.jvs_read_coin,
        }
    }

    pub unsafe fn jvs_read_coin(&self) -> u16 {
        self.ensure_jvs_init();
        let mut total = 0u16;
        (self.jvs_read_coin)(&mut total);
        total
    }

    pub unsafe fn slider_start(&self, callback: SliderCallbackRaw) {
        self.ensure_slider_init();
        (self.slider_start)(callback);
    }

    pub unsafe fn slider_stop(&self) {
        (self.slider_stop)();
    }

    pub unsafe fn slider_set_leds(&self, rgb: *const u8) {
        if let Some(func) = self.slider_set_leds {
            func(rgb);
        }
    }

    pub unsafe fn led_set_colors(&self, board: u8, rgb: *mut u8) {
        self.ensure_led_init();
        if let Some(func) = self.led_set_colors {
            func(board, rgb);
        }
    }
}
