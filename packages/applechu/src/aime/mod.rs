use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;

use crate::config::Config;
use crate::iohook::uart;
use crate::iohook::{self, Irp, IrpOp};
use crate::util::api::Api;

pub mod external;
mod sg_nfc;

use self::external::{AimeIoVfdState, ExternalAimeIo};
use self::sg_nfc::{SgNfcConfig, SgNfcDevice};

const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_FUNCTION: u32 = 1;

pub(crate) static NUL_FILENAME: [u16; 4] = [b'N' as u16, b'U' as u16, b'L' as u16, 0];

// 所有架构都把串口重定向到以共享、重叠方式打开的 NUL 句柄
const EMULATED_OPEN_SHARE: u32 = 3; // FILE_SHARE_READ | FILE_SHARE_WRITE
const EMULATED_OPEN_FLAGS: u32 = 0x4000_0000; // FILE_FLAG_OVERLAPPED

static AIME_PORT: Mutex<u32> = Mutex::new(4);

static AIME_READER: Mutex<Option<AimeReader>> = Mutex::new(None);
static AIME_DEVICE: Mutex<Option<SgNfcDevice>> = Mutex::new(None);
// 同一串口上的 UART、NFC 和 RGB LED 状态必须在一次 IRP 处理期间保持一致
static SG_READER_LOCK: Mutex<()> = Mutex::new(());
static AIME_FD: AtomicUsize = AtomicUsize::new(0);
// 外部 Aime IO 只初始化一次，串口句柄可反复开关
static AIME_START_STATUS: Mutex<Option<i32>> = Mutex::new(None);
static EXTERNAL: Lazy<Mutex<Option<ExternalAimeIo>>> = Lazy::new(|| Mutex::new(None));

crate::config_section! {
    pub(crate) struct AimeSectionConfig => AIME_CONFIG_SECTION {
        section: "Aime",
        order: 300,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "Aime 读卡器模拟",
        fields: {
            // 两种机台默认使用 COM4，同时保留分模式覆盖项
            pub cvt_port: u32 = 4,
            key: "cvtPort",
            comment: "CVT 模式串口号";
            pub sp_port: u32 = 4,
            key: "spPort",
            comment: "SP 模式串口号";
            pub high_baudrate: bool = true,
            key: "highBaud",
            comment: "使用 115200 高波特率（Chunithm 默认需要）";
            pub aime_path: String = String::from("DEVICE\\aime.txt"),
            key: "aimePath",
            comment: "Aime 卡号文件";
            pub felica_path: String = String::from("DEVICE\\felica.txt"),
            key: "felicaPath",
            comment: "FeliCa 卡号文件";
            pub authdata_path: String = String::from("DEVICE\\authdata.bin"),
            key: "authdataPath",
            comment: "认证数据文件";
            pub aime_gen: bool = true,
            key: "aimeGen",
            comment: "缺少 Aime 卡号时自动生成";
            pub felica_gen: bool = false,
            key: "felicaGen",
            comment: "缺少 FeliCa 卡号时自动生成";
            pub scan: i32 = 0x0D,
            comment: "读卡按键的虚拟键码";
            // 0 表示按机台模式选择：CVT=Gen2，SP=Gen3
            pub gen: u8 = 0,
            comment: "读卡器代数";
            pub proxy_flag: u8 = 2,
            key: "proxyFlag",
            comment: "读卡代理标志";
        }
    }
}

crate::config_section! {
    pub(crate) struct AimeIoConfig => AIME_IO_CONFIG_SECTION {
        section: "AimeIo",
        order: 301,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "外部 Aime IO DLL",
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

#[derive(Clone)]
pub struct AimeConfig {
    pub cvt_port: u32,
    pub sp_port: u32,
    pub high_baudrate: bool,
    pub aime_path: PathBuf,
    pub felica_path: PathBuf,
    pub authdata_path: PathBuf,
    pub aime_gen: bool,
    pub felica_gen: bool,
    pub scan_key: i32,
    pub gen: u8,
    pub proxy_flag: u8,
}

pub struct AimeReader {
    config: AimeConfig,
    aime_id: Option<[u8; 10]>,
    felica_id: Option<u64>,
    radio_on: bool,
    update_mode: bool,
}

impl AimeReader {
    pub fn new(config: AimeConfig) -> Self {
        Self {
            config,
            aime_id: None,
            felica_id: None,
            radio_on: true,
            update_mode: false,
        }
    }

    /// 刷新一次卡片存在状态
    pub fn poll(&mut self) -> i32 {
        self.aime_id = None;
        self.felica_id = None;
        if !self.radio_on || !is_scan_key_down(self.config.scan_key) {
            return iohook::S_OK;
        }

        if let Some(status) = external_call(|external| unsafe { external.poll() }) {
            if status < 0 {
                return status;
            }
            if let Ok(guard) = EXTERNAL.lock() {
                if let Some(external) = guard.as_ref() {
                    // 先尝试 Aime，再尝试 FeliCa，getter 不应再次 poll
                    self.aime_id = unsafe { external.get_aime_id() };
                    if self.aime_id.is_none() {
                        self.felica_id = unsafe { external.get_felica_id() };
                    }
                }
            }
            return iohook::S_OK;
        }

        self.aime_id = read_or_generate_id(&self.config.aime_path, 10, self.config.aime_gen)
            .and_then(|bytes| bytes.try_into().ok());
        if self.aime_id.is_none() {
            self.felica_id =
                read_or_generate_id(&self.config.felica_path, 8, self.config.felica_gen).and_then(
                    |bytes| {
                        let bytes: [u8; 8] = bytes.try_into().ok()?;
                        Some(
                            bytes
                                .into_iter()
                                .fold(0u64, |acc, byte| (acc << 8) | u64::from(byte)),
                        )
                    },
                );
        }
        iohook::S_OK
    }

    pub fn aime_id(&self) -> Option<[u8; 10]> {
        self.aime_id
    }

    pub fn felica_id(&self) -> Option<u64> {
        self.felica_id
    }

    pub fn radio_on(&mut self) -> i32 {
        self.radio_on = true;
        self.update_mode = false;
        external_call(|external| unsafe { external.radio_on() }).unwrap_or(iohook::S_OK)
    }

    pub fn radio_off(&mut self) -> i32 {
        self.radio_on = false;
        external_call(|external| unsafe { external.radio_off() }).unwrap_or(iohook::S_OK)
    }

    pub fn enter_update_mode(&mut self) -> i32 {
        self.update_mode = true;
        external_call(|external| unsafe { external.to_update_mode() }).unwrap_or(iohook::S_OK)
    }

    pub fn read_mifare_uid(&self) -> Option<[u8; 4]> {
        self.aime_id?;
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                return unsafe { external.get_mifare_uid() };
            }
        }
        None
    }

    pub fn mifare_read_block(&self, uid: &[u8], block_no: u8, block: &mut [u8]) -> bool {
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                return unsafe { external.mifare_read_block(uid, block_no, block) == 0 };
            }
        }
        false
    }

    pub fn felica_transact(&self, req: &[u8], res: &mut [u8]) -> Option<usize> {
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                return unsafe { external.felica_transact(req, res) };
            }
        }
        None
    }
}

#[applechu_macros::config_section(stage = Device, order = 60)]
pub fn init(api: &Api, config: &Config, section: &AimeSectionConfig) {
    let cfg = load_config(section, config.base_dir());
    let is_sp = crate::system_config::is_sp_mode(config);
    let port = if is_sp { cfg.sp_port } else { cfg.cvt_port };
    let gen = reader_generation(cfg.gen, is_sp);

    let path = config
        .section::<AimeIoConfig>()
        .filter(|config| config.enabled)
        .map_or_else(String::new, |config| dll_path(&config));
    if !path.is_empty() {
        match unsafe { ExternalAimeIo::load(&path) } {
            Ok(external) => {
                let version = external.api_version;
                if let Ok(mut guard) = EXTERNAL.lock() {
                    *guard = Some(external);
                }
                api.log_info(&format!(
                    "External Aime IO loaded: {path}, API {version:#06x}"
                ));
            }
            Err(err) => {
                api.log_warn(&format!(
                    "External Aime IO failed to load; using built-in emulation: {err}"
                ));
            }
        }
    }

    if let Ok(mut aime_port) = AIME_PORT.lock() {
        *aime_port = port;
    }
    if let Ok(mut reader) = AIME_READER.lock() {
        *reader = Some(AimeReader::new(cfg.clone()));
    }
    if let Ok(mut status) = AIME_START_STATUS.lock() {
        *status = None;
    }
    if let Ok(mut device) = AIME_DEVICE.lock() {
        *device = Some(SgNfcDevice::new(SgNfcConfig {
            gen,
            proxy_flag: cfg.proxy_flag,
            authdata_path: cfg.authdata_path.clone(),
        }));
    }
    unsafe {
        iohook::push_handler(aime_irp_handler);
    }
    api.log_info(&format!(
        "Aime reader emulation enabled on COM{port}, generation {gen}"
    ));
}

unsafe fn aime_irp_handler(irp: &mut Irp) -> i32 {
    let Ok(_reader_guard) = SG_READER_LOCK.lock() else {
        return iohook::E_FAIL;
    };
    let port = aime_port();
    if irp.op == IrpOp::Open {
        if !matches_com_port(irp, port) {
            return iohook::invoke_next(irp);
        }
        if AIME_FD.load(Ordering::SeqCst) != 0 {
            return iohook::hresult_from_win32(ERROR_ACCESS_DENIED);
        }
        let status = start_backend_once();
        if status < 0 {
            return status;
        }

        // 将目标改为 NUL 后继续钩子链，以取得有效的重叠 I/O 句柄
        irp.open_filename_w = NUL_FILENAME.as_ptr();
        irp.open_filename_a = ptr::null();
        irp.open_access = 0xC000_0000; // GENERIC_READ | GENERIC_WRITE
        irp.open_share = EMULATED_OPEN_SHARE;
        irp.open_security = ptr::null();
        irp.open_creation = 3; // OPEN_EXISTING
        irp.open_flags = EMULATED_OPEN_FLAGS;
        irp.open_template = ptr::null_mut();

        let result = iohook::invoke_next(irp);
        if result >= 0 {
            let handle = crate::util::win32::handle_value(irp.fd);
            uart::bind_handle(handle, port);
            let high_baudrate = AIME_READER
                .lock()
                .ok()
                .and_then(|reader| reader.as_ref().map(|reader| reader.config.high_baudrate))
                .unwrap_or(true);
            // 默认启用 115200，显式关闭时使用 38400
            uart::set_baud_rate(handle, if high_baudrate { 115_200 } else { 38_400 });
            AIME_FD.store(handle, Ordering::SeqCst);
        } else {
            if let Some(api) = crate::util::api::API.get() {
                api.log_error(&format!(
                    "Aime reader failed to open COM{port}: {result:#010x}"
                ));
            }
        }
        return result;
    }

    let my_fd = AIME_FD.load(Ordering::SeqCst);
    if my_fd == 0 || crate::util::win32::handle_value(irp.fd) != my_fd {
        return iohook::invoke_next(irp);
    }

    if irp.op == IrpOp::Close {
        AIME_FD.store(0, Ordering::SeqCst);
        uart::unbind_handle(my_fd);
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Read => uart::uart_handle_irp(irp),
        IrpOp::Write => write_aime(irp),
        IrpOp::Ioctl => uart::device_io_control(
            crate::util::win32::handle_value(irp.fd),
            irp.ioctl,
            irp.ioctl_in,
            irp.ioctl_in_nbytes,
            irp.ioctl_out,
            irp.ioctl_out_nbytes,
            irp.out_nbytes,
        ),
        IrpOp::Fsync => iohook::S_OK,
        IrpOp::Seek | IrpOp::Open | IrpOp::Close => {
            iohook::hresult_from_win32(ERROR_INVALID_FUNCTION)
        }
    }
}

fn start_backend_once() -> i32 {
    let Ok(mut started) = AIME_START_STATUS.lock() else {
        return iohook::E_FAIL;
    };
    if let Some(status) = *started {
        return status;
    }
    if let Some(api) = crate::util::api::API.get() {
        api.log_info("Aime reader backend starting");
    }
    let status = EXTERNAL
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|external| unsafe { external.init() }))
        .unwrap_or(iohook::S_OK);
    *started = Some(status);
    if status < 0 {
        if let Some(api) = crate::util::api::API.get() {
            api.log_error(&format!(
                "Aime reader backend failed to start: {status:#010x}"
            ));
        }
    }
    status
}

unsafe fn write_aime(irp: &mut Irp) -> i32 {
    let hr = uart::uart_handle_irp(irp);
    if hr < 0 {
        return hr;
    }
    let handle = crate::util::win32::handle_value(irp.fd);
    let Some(mut written) = uart::take_written(handle) else {
        return iohook::E_FAIL;
    };
    let mut readable = Vec::new();
    if let (Ok(mut device), Ok(mut reader)) = (AIME_DEVICE.lock(), AIME_READER.lock()) {
        if let (Some(device), Some(reader)) = (device.as_mut(), reader.as_mut()) {
            device.process(reader, &mut written, &mut readable);
        }
    } else {
        return iohook::E_FAIL;
    }
    if !uart::restore_written(handle, written) {
        return iohook::E_FAIL;
    }
    if !readable.is_empty() && !uart::push_readable(handle, &readable) {
        return iohook::E_FAIL;
    }
    hr
}

unsafe fn matches_com_port(irp: &Irp, port_no: u32) -> bool {
    uart::parse_com_a(irp.open_filename_a).or_else(|| uart::parse_com_w(irp.open_filename_w))
        == Some(port_no)
}

fn aime_port() -> u32 {
    AIME_PORT.lock().map_or(4, |p| *p)
}

fn load_config(config: &AimeSectionConfig, base_dir: impl AsRef<Path>) -> AimeConfig {
    AimeConfig {
        cvt_port: config.cvt_port,
        sp_port: config.sp_port,
        high_baudrate: config.high_baudrate,
        aime_path: resolve_path(base_dir.as_ref(), &config.aime_path),
        felica_path: resolve_path(base_dir.as_ref(), &config.felica_path),
        authdata_path: resolve_path(base_dir.as_ref(), &config.authdata_path),
        aime_gen: config.aime_gen,
        felica_gen: config.felica_gen,
        scan_key: config.scan,
        gen: config.gen,
        proxy_flag: config.proxy_flag,
    }
}

fn reader_generation(configured: u8, is_sp: bool) -> u8 {
    if configured == 0 {
        if is_sp {
            3
        } else {
            2
        }
    } else {
        configured.clamp(1, 3)
    }
}

pub(super) fn external_call<F>(call: F) -> Option<i32>
where
    F: FnOnce(&ExternalAimeIo) -> i32,
{
    let guard = EXTERNAL.lock().ok()?;
    guard.as_ref().map(call)
}

fn dll_path(config: &AimeIoConfig) -> String {
    let arch_path = if cfg!(target_pointer_width = "64") {
        &config.path64
    } else {
        &config.path32
    };
    if !arch_path.is_empty() {
        return arch_path.clone();
    }
    config.path.clone()
}

fn resolve_path(base_dir: impl AsRef<Path>, path: &str) -> PathBuf {
    if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        base_dir.as_ref().join(path)
    }
}

pub fn vfd_set_text(text: &[u8], state: &AimeIoVfdState) {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            unsafe { external.vfd_set_text(text, state) };
        }
    }
}

pub fn vfd_set_state(state: &AimeIoVfdState) {
    if let Ok(guard) = EXTERNAL.lock() {
        if let Some(external) = guard.as_ref() {
            unsafe { external.vfd_set_state(state) };
        }
    }
}

fn read_or_generate_id(path: &Path, len: usize, generate: bool) -> Option<Vec<u8>> {
    if let Ok(text) = fs::read_to_string(path) {
        if let Some(bytes) = parse_hex_id(&text, len) {
            return Some(bytes);
        }
    }
    generate.then(|| generate_id_file(path, len))?
}

fn parse_hex_id(text: &str, len: usize) -> Option<Vec<u8>> {
    let digits: String = text.chars().filter(|ch| ch.is_ascii_hexdigit()).collect();
    if digits.len() != len * 2 {
        return None;
    }
    let mut id = vec![0; len];
    for (idx, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[idx * 2..idx * 2 + 2], 16).ok()?;
    }
    Some(id)
}

fn generate_id_file(path: &Path, len: usize) -> Option<Vec<u8>> {
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    let mut out = vec![0; len];
    for byte in &mut out {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        *byte = ((seed >> 32) & 0xFF) as u8;
    }
    if len == 10 {
        for byte in &mut out {
            *byte = ((*byte >> 4) % 10) << 4 | (*byte % 10);
        }
        if out[0] >> 4 == 3 {
            out[0] = 0x10;
        }
    } else if let Some(first) = out.first_mut() {
        *first &= 0x0F;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let text = out
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    let _ = fs::write(path, format!("{text}\n"));
    Some(out)
}

#[cfg(windows)]
fn is_scan_key_down(vk: i32) -> bool {
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) & 0x8000u16 as i16
            != 0
    }
}

#[cfg(not(windows))]
fn is_scan_key_down(_vk: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{reader_generation, AimeSectionConfig};

    #[test]
    fn default_card_files_use_device_directory() {
        let config = AimeSectionConfig::default();
        assert_eq!(config.aime_path, r"DEVICE\aime.txt");
        assert_eq!(config.felica_path, r"DEVICE\felica.txt");
    }

    #[test]
    fn default_reader_generation_follows_cabinet_mode() {
        assert_eq!(reader_generation(0, true), 3);
        assert_eq!(reader_generation(0, false), 2);
    }

    #[test]
    fn explicit_reader_generation_overrides_cabinet_mode() {
        assert_eq!(reader_generation(1, true), 1);
        assert_eq!(reader_generation(2, true), 2);
        assert_eq!(reader_generation(3, false), 3);
    }
}
