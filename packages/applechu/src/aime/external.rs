use std::ffi::c_void;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{GetLastError, HMODULE};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

fn string_to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

const S_OK: i32 = 0;

type GetApiVersionFn = unsafe extern "C" fn() -> u16;
type InitFn = unsafe extern "C" fn() -> i32;
type NfcPollFn = unsafe extern "C" fn(u8) -> i32;
type NfcGetAimeIdFn = unsafe extern "C" fn(u8, *mut u8, usize) -> i32;
type NfcGetFelicaIdFn = unsafe extern "C" fn(u8, *mut u64) -> i32;
type NfcGetMifareUidFn = unsafe extern "C" fn(u8, *mut u8, usize) -> i32;
type NfcMifareSelectFn = unsafe extern "C" fn(u8, *const u8, usize) -> i32;
type NfcMifareSetKeyFn = unsafe extern "C" fn(u8, u8, *const u8, usize) -> i32;
type NfcMifareAuthenticateFn = unsafe extern "C" fn(u8, u8, *const u8, usize) -> i32;
type NfcMifareReadBlockFn = unsafe extern "C" fn(u8, *const u8, usize, u8, *mut u8, usize) -> i32;
type NfcFelicaTransactFn =
    unsafe extern "C" fn(u8, *const u8, usize, *mut u8, usize, *mut usize) -> i32;
type NfcUnitFn = unsafe extern "C" fn(u8) -> i32;
type NfcSendHexDataFn = unsafe extern "C" fn(u8, *const u8, usize, *mut u8) -> i32;
type LedSetColorFn = unsafe extern "C" fn(u8, u8, u8, u8);
type VfdSetTextFn = unsafe extern "C" fn(*const u8, usize, *const AimeIoVfdState);
type VfdSetStateFn = unsafe extern "C" fn(*const AimeIoVfdState);

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AimeIoVfdState {
    pub encoding: u8,
    pub text_speed: u8,
    pub scroll_enabled: u8,
    pub h_scroll: u16,
    pub cursor_x: u16,
    pub cursor_y: u8,
    pub wnd_x0: u16,
    pub wnd_y0: u8,
    pub wnd_x1: u16,
    pub wnd_y1: u8,
    pub rotate: u8,
    pub brightness: u8,
    pub screen_on: u8,
    pub clear_seq: u32,
}

pub struct ExternalAimeIo {
    pub api_version: u16,
    init: InitFn,
    nfc_poll: NfcPollFn,
    nfc_get_aime_id: NfcGetAimeIdFn,
    nfc_get_felica_id: Option<NfcGetFelicaIdFn>,
    nfc_get_mifare_uid: Option<NfcGetMifareUidFn>,
    nfc_mifare_select: Option<NfcMifareSelectFn>,
    nfc_mifare_set_key: Option<NfcMifareSetKeyFn>,
    nfc_mifare_authenticate: Option<NfcMifareAuthenticateFn>,
    nfc_mifare_read_block: Option<NfcMifareReadBlockFn>,
    nfc_felica_transact: Option<NfcFelicaTransactFn>,
    nfc_radio_on: Option<NfcUnitFn>,
    nfc_radio_off: Option<NfcUnitFn>,
    nfc_to_update_mode: Option<NfcUnitFn>,
    nfc_send_hex_data: Option<NfcSendHexDataFn>,
    led_set_color: Option<LedSetColorFn>,
    vfd_set_text: Option<VfdSetTextFn>,
    vfd_set_state: Option<VfdSetStateFn>,
    start_status: OnceLock<i32>,
}

unsafe impl Send for ExternalAimeIo {}
unsafe impl Sync for ExternalAimeIo {}

unsafe fn resolve(module: HMODULE, name: &[u8]) -> Option<*const c_void> {
    GetProcAddress(module, name.as_ptr()).map(|proc| proc as *const c_void)
}

impl ExternalAimeIo {
    pub unsafe fn load(path: &str) -> Result<Self, String> {
        let wide = string_to_wide(path);
        let module = LoadLibraryW(wide.as_ptr());
        if module.is_null() {
            return Err(format!(
                "LoadLibraryW failed for {path}, Win32 error {}",
                GetLastError()
            ));
        }

        let api_version = resolve(module, b"aime_io_get_api_version\0")
            .map(|proc| {
                let func: GetApiVersionFn = std::mem::transmute(proc);
                func()
            })
            .unwrap_or(0x0100);

        let init: InitFn =
            std::mem::transmute(resolve(module, b"aime_io_init\0").ok_or("missing aime_io_init")?);
        let nfc_poll: NfcPollFn = std::mem::transmute(
            resolve(module, b"aime_io_nfc_poll\0").ok_or("missing aime_io_nfc_poll")?,
        );
        let nfc_get_aime_id: NfcGetAimeIdFn = std::mem::transmute(
            resolve(module, b"aime_io_nfc_get_aime_id\0")
                .ok_or("missing aime_io_nfc_get_aime_id")?,
        );
        let nfc_get_felica_id =
            resolve(module, b"aime_io_nfc_get_felica_id\0").map(|proc| std::mem::transmute(proc));
        let nfc_get_mifare_uid =
            resolve(module, b"aime_io_nfc_get_mifare_uid\0").map(|proc| std::mem::transmute(proc));
        let nfc_mifare_select =
            resolve(module, b"aime_io_nfc_mifare_select\0").map(|proc| std::mem::transmute(proc));
        let nfc_mifare_set_key =
            resolve(module, b"aime_io_nfc_mifare_set_key\0").map(|proc| std::mem::transmute(proc));
        let nfc_mifare_authenticate = resolve(module, b"aime_io_nfc_mifare_authenticate\0")
            .map(|proc| std::mem::transmute(proc));
        let nfc_mifare_read_block = resolve(module, b"aime_io_nfc_mifare_read_block\0")
            .map(|proc| std::mem::transmute(proc));
        let nfc_felica_transact =
            resolve(module, b"aime_io_nfc_felica_transact\0").map(|proc| std::mem::transmute(proc));
        let nfc_radio_on =
            resolve(module, b"aime_io_nfc_radio_on\0").map(|proc| std::mem::transmute(proc));
        let nfc_radio_off =
            resolve(module, b"aime_io_nfc_radio_off\0").map(|proc| std::mem::transmute(proc));
        let nfc_to_update_mode =
            resolve(module, b"aime_io_nfc_to_update_mode\0").map(|proc| std::mem::transmute(proc));
        let nfc_send_hex_data =
            resolve(module, b"aime_io_nfc_send_hex_data\0").map(|proc| std::mem::transmute(proc));
        let led_set_color =
            resolve(module, b"aime_io_led_set_color\0").map(|proc| std::mem::transmute(proc));
        let vfd_set_text =
            resolve(module, b"aime_io_vfd_set_text\0").map(|proc| std::mem::transmute(proc));
        let vfd_set_state =
            resolve(module, b"aime_io_vfd_set_state\0").map(|proc| std::mem::transmute(proc));

        Ok(Self {
            api_version,
            init,
            nfc_poll,
            nfc_get_aime_id,
            nfc_get_felica_id,
            nfc_get_mifare_uid,
            nfc_mifare_select,
            nfc_mifare_set_key,
            nfc_mifare_authenticate,
            nfc_mifare_read_block,
            nfc_felica_transact,
            nfc_radio_on,
            nfc_radio_off,
            nfc_to_update_mode,
            nfc_send_hex_data,
            led_set_color,
            vfd_set_text,
            vfd_set_state,
            start_status: OnceLock::new(),
        })
    }

    pub unsafe fn init(&self) -> i32 {
        *self.start_status.get_or_init(|| (self.init)())
    }

    unsafe fn ensure_init(&self) {
        let _ = self.init();
    }

    pub unsafe fn read_aime_id(&self) -> Option<[u8; 10]> {
        self.ensure_init();
        (self.nfc_poll)(0);
        self.get_aime_id()
    }

    pub unsafe fn get_aime_id(&self) -> Option<[u8; 10]> {
        self.ensure_init();
        let mut luid = [0u8; 10];
        if (self.nfc_get_aime_id)(0, luid.as_mut_ptr(), luid.len()) == S_OK {
            return Some(luid);
        }
        None
    }

    pub unsafe fn read_felica_id(&self) -> Option<u64> {
        self.ensure_init();
        (self.nfc_poll)(0);
        self.get_felica_id()
    }

    pub unsafe fn get_felica_id(&self) -> Option<u64> {
        self.ensure_init();
        let func = self.nfc_get_felica_id?;
        let mut idm = 0u64;
        if func(0, &mut idm) == S_OK {
            return Some(idm);
        }
        None
    }

    pub unsafe fn poll(&self) -> i32 {
        self.ensure_init();
        (self.nfc_poll)(0)
    }

    pub unsafe fn get_mifare_uid(&self) -> Option<[u8; 4]> {
        self.ensure_init();
        let func = self.nfc_get_mifare_uid?;
        let mut uid = [0u8; 4];
        if func(0, uid.as_mut_ptr(), uid.len()) == S_OK {
            return Some(uid);
        }
        None
    }

    pub unsafe fn mifare_select(&self, uid: &[u8]) -> i32 {
        self.ensure_init();
        self.nfc_mifare_select
            .map_or(0, |func| func(0, uid.as_ptr(), uid.len()))
    }

    pub unsafe fn mifare_set_key(&self, key_type: u8, key: &[u8]) -> i32 {
        self.ensure_init();
        self.nfc_mifare_set_key
            .map_or(0, |func| func(0, key_type, key.as_ptr(), key.len()))
    }

    pub unsafe fn mifare_authenticate(&self, key_type: u8, payload: &[u8]) -> i32 {
        self.ensure_init();
        self.nfc_mifare_authenticate
            .map_or(0, |func| func(0, key_type, payload.as_ptr(), payload.len()))
    }

    pub unsafe fn felica_transact(&self, req: &[u8], res: &mut [u8]) -> Option<usize> {
        self.ensure_init();
        let func = self.nfc_felica_transact?;
        let mut written = 0usize;
        if func(
            0,
            req.as_ptr(),
            req.len(),
            res.as_mut_ptr(),
            res.len(),
            &mut written,
        ) == S_OK
        {
            return Some(written);
        }
        None
    }

    pub unsafe fn to_update_mode(&self) -> i32 {
        self.ensure_init();
        self.nfc_to_update_mode.map_or(0, |func| func(0))
    }

    pub unsafe fn led_set_color(&self, r: u8, g: u8, b: u8) {
        if let Some(func) = self.led_set_color {
            func(0, r, g, b);
        }
    }

    pub unsafe fn radio_on(&self) -> i32 {
        self.ensure_init();
        self.nfc_radio_on.map_or(0, |func| func(0))
    }

    pub unsafe fn radio_off(&self) -> i32 {
        self.ensure_init();
        self.nfc_radio_off.map_or(0, |func| func(0))
    }

    pub unsafe fn send_hex_data(&self, payload: &[u8], status_out: *mut u8) -> i32 {
        self.ensure_init();
        self.nfc_send_hex_data.map_or(0, |func| {
            func(0, payload.as_ptr(), payload.len(), status_out)
        })
    }

    pub unsafe fn mifare_read_block(&self, uid: &[u8], block_no: u8, block: &mut [u8]) -> i32 {
        self.ensure_init();
        self.nfc_mifare_read_block.map_or(1, |func| {
            func(
                0,
                uid.as_ptr(),
                uid.len(),
                block_no,
                block.as_mut_ptr(),
                block.len(),
            )
        })
    }

    pub unsafe fn vfd_set_text(&self, text: &[u8], state: &AimeIoVfdState) {
        if let Some(func) = self.vfd_set_text {
            self.ensure_init();
            func(text.as_ptr(), text.len(), state);
        }
    }

    pub unsafe fn vfd_set_state(&self, state: &AimeIoVfdState) {
        if let Some(func) = self.vfd_set_state {
            self.ensure_init();
            func(state);
        }
    }
}
