pub mod chu2to3;
pub mod config;
pub mod external;
mod led_output;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use once_cell::sync::Lazy;

use crate::config::Config;
use crate::util::api::Api;

use self::config::ChuniIoConfig;
use self::external::ExternalChuniIo;
use self::led_output::LedOutputConfig;

crate::config_section! {
    pub(crate) struct ExternalChuniIoConfig => CHUNI_IO_CONFIG_SECTION {
        section: "ChuniIo",
        order: 309,
        default_enabled: true,
        always_enabled: true,
        hidden: false,
        comment: "外部 ChuniIO DLL",
        fields: {
            pub path: String = String::new(),
            comment: "所有架构共用的 DLL 路径";
            pub path32: String = String::new(),
            comment: "32 位 DLL 路径";
            pub path64: String = String::new(),
            comment: "64 位 DLL 路径";
        }
    }
}

type SliderCallback = Arc<dyn Fn([u8; 32]) + Send + Sync + 'static>;

pub const OPBTN_TEST: u8 = 0x01;
pub const OPBTN_SERVICE: u8 = 0x02;
pub const OPBTN_COIN: u8 = 0x04;

pub const BUILTIN_API_VERSION: u16 = 0x0102;

static EXTERNAL: Lazy<Mutex<Option<ExternalChuniIo>>> = Lazy::new(|| Mutex::new(None));
static EXTERNAL_SLIDER_CB: Mutex<Option<SliderCallback>> = Mutex::new(None);

struct ChuniIoBackend {
    config: ChuniIoConfig,
    hand_pos: u8,
    coin_held: bool,
    coins: u16,
    slider_stop: Arc<AtomicBool>,
    slider_thread: Option<JoinHandle<()>>,
}

static BACKEND: Lazy<Mutex<Option<ChuniIoBackend>>> = Lazy::new(|| Mutex::new(None));

pub fn init(api: &Api, config: &Config) {
    let led_config = config
        .section::<LedOutputConfig>()
        .map_or_else(LedOutputConfig::default, |value| (*value).clone());
    led_output::init(led_config);
    let (path, single_dll) = chuniio_path(config);
    if !path.is_empty() {
        match unsafe { ExternalChuniIo::load(&path) } {
            Ok(external) => {
                let version = external.api_version;
                // Single-DLL `path` mode mirrors segatools' chu2to3 engine: the
                // 32-bit chusanApp side must publish JVS state into the
                // `Local\\Chu2to3Shmem` shared memory so the chusanhook_x64
                // injected into amdaemon can read it. Without this, amdaemon's
                // OpenFileMapping fails and no JVS input reaches the game.
                if single_dll {
                    let fns = external.jvs_raw_fns();
                    chu2to3::start(api, fns);
                }
                if let Ok(mut guard) = EXTERNAL.lock() {
                    *guard = Some(external);
                }
                api.log_info(&format!(
                    "ChuniIo: external IO DLL loaded: {path} (API {version:#06x})"
                ));
                return;
            }
            Err(err) => {
                api.log_warn(&format!(
                    "ChuniIo: external IO DLL failed ({err}), falling back to keyboard"
                ));
            }
        }
    }

    if jvs_init(config).is_ok() {
        api.log_info("ChuniIo keyboard backend initialized");
    }
}

fn external_active() -> bool {
    EXTERNAL
        .lock()
        .map(|guard| guard.is_some())
        .unwrap_or(false)
}

pub fn jvs_init(config: &Config) -> Result<(), i32> {
    let backend = ChuniIoBackend {
        config: ChuniIoConfig::load(config),
        hand_pos: 0,
        coin_held: false,
        coins: 0,
        slider_stop: Arc::new(AtomicBool::new(false)),
        slider_thread: None,
    };

    BACKEND.lock().map_err(|_| -1)?.replace(backend);
    Ok(())
}

pub fn jvs_poll(opbtn: &mut u8, beams: &mut u8) {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            unsafe { external.jvs_poll(opbtn, beams) };
            return;
        }
    }

    let Ok(mut guard) = BACKEND.lock() else {
        return;
    };
    let Some(backend) = guard.as_mut() else {
        return;
    };

    if is_key_down(backend.config.vk_test) {
        *opbtn |= OPBTN_TEST;
    }
    if is_key_down(backend.config.vk_service) {
        *opbtn |= OPBTN_SERVICE;
    }

    if backend.config.vk_ir_emu != 0 {
        if is_key_down(backend.config.vk_ir_emu) {
            backend.hand_pos = (backend.hand_pos + 1).min(6);
        } else {
            backend.hand_pos = backend.hand_pos.saturating_sub(1);
        }

        for idx in 0..6 {
            if backend.hand_pos > idx {
                *beams |= 1 << idx;
            }
        }
    } else {
        for (idx, key) in backend.config.vk_ir.iter().enumerate() {
            if is_key_down(*key) {
                *beams |= 1 << idx;
            } else {
                *beams &= !(1 << idx);
            }
        }
    }
}

pub fn jvs_read_coin_counter() -> u16 {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            return unsafe { external.jvs_read_coin() };
        }
    }
    let Ok(mut guard) = BACKEND.lock() else {
        return 0;
    };
    let Some(backend) = guard.as_mut() else {
        return 0;
    };
    let pressed = is_key_down(backend.config.vk_coin);
    if pressed && !backend.coin_held {
        backend.coins = backend.coins.wrapping_add(1);
    }
    backend.coin_held = pressed;
    backend.coins
}

pub fn effective_api_version() -> u16 {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            return external.api_version;
        }
    }
    BUILTIN_API_VERSION
}

pub fn slider_init() -> Result<(), i32> {
    Ok(())
}

pub fn slider_start(callback: SliderCallback) {
    if external_active() {
        if let Ok(mut cb) = EXTERNAL_SLIDER_CB.lock() {
            *cb = Some(callback);
        }
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                unsafe { external.slider_start(external_slider_trampoline) };
            }
        }
        return;
    }

    let Ok(mut guard) = BACKEND.lock() else {
        return;
    };
    let Some(backend) = guard.as_mut() else {
        return;
    };
    if backend.slider_thread.is_some() {
        return;
    }

    let keys = backend.config.vk_cell;
    let stop = Arc::new(AtomicBool::new(false));
    backend.slider_stop = stop.clone();
    backend.slider_thread = Some(thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let mut pressure = [0u8; 32];
            for (idx, key) in keys.iter().enumerate() {
                if is_key_down(*key) {
                    pressure[idx] = 128;
                }
            }
            callback(pressure);
            thread::sleep(Duration::from_millis(1));
        }
    }));
}

pub fn slider_stop() {
    if external_active() {
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                unsafe { external.slider_stop() };
            }
        }
        if let Ok(mut cb) = EXTERNAL_SLIDER_CB.lock() {
            *cb = None;
        }
        return;
    }

    let Ok(mut guard) = BACKEND.lock() else {
        return;
    };
    let Some(backend) = guard.as_mut() else {
        return;
    };
    backend.slider_stop.store(true, Ordering::Relaxed);
    if let Some(thread) = backend.slider_thread.take() {
        let _ = thread.join();
    }
}

unsafe extern "C" fn external_slider_trampoline(state: *const u8) {
    if state.is_null() {
        return;
    }
    let mut pressure = [0u8; 32];
    std::ptr::copy_nonoverlapping(state, pressure.as_mut_ptr(), 32);
    if let Ok(guard) = EXTERNAL_SLIDER_CB.lock() {
        if let Some(callback) = guard.as_ref() {
            callback(pressure);
        }
    }
}

pub fn slider_set_leds(rgb: &[u8]) {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            unsafe { external.slider_set_leds(rgb.as_ptr()) };
            return;
        }
    }
    led_output::update(2, rgb);
}

pub fn led_set_colors(board: u8, rgb: &mut [u8]) {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            unsafe { external.led_set_colors(board, rgb.as_mut_ptr()) };
            return;
        }
    }
    led_output::update(board, rgb);
}

/// Resolve the external chuniio DLL path.
///
/// Returns `(path, single_dll)`. `single_dll` is true when the single-DLL
/// `path` setting is used, which triggers the chu2to3 shared-memory bridge so
/// the amdaemon-side chusanhook_x64 can consume JVS state. The split
/// `path32`/`path64` form is the dual-DLL mode (each process loads its own
/// matching DLL) and does not use chu2to3.
fn chuniio_path(config: &Config) -> (String, bool) {
    let config = config
        .section::<ExternalChuniIoConfig>()
        .map_or_else(ExternalChuniIoConfig::default, |value| (*value).clone());
    let path = config.path;
    if !path.is_empty() {
        return (path, true);
    }
    let split = if cfg!(target_pointer_width = "64") {
        config.path64
    } else {
        config.path32
    };
    (split, false)
}

#[cfg(windows)]
fn is_key_down(vk: i32) -> bool {
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) & 0x8000u16 as i16
            != 0
    }
}

#[cfg(not(windows))]
fn is_key_down(_vk: i32) -> bool {
    false
}
