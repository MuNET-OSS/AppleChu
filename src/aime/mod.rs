use std::fs;
use std::path::{Path, PathBuf};
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

static AIME_PORT: Mutex<u32> = Mutex::new(4);

static AIME_READER: Mutex<Option<AimeReader>> = Mutex::new(None);
static AIME_DEVICE: Mutex<Option<SgNfcDevice>> = Mutex::new(None);
static AIME_FD: AtomicUsize = AtomicUsize::new(0);
static EXTERNAL: Lazy<Mutex<Option<ExternalAimeIo>>> = Lazy::new(|| Mutex::new(None));

crate::config_section! {
    pub(crate) struct AimeSectionConfig => AIME_CONFIG_SECTION {
        section: "Aime",
        order: 300,
        default_enabled: true,
        always_enabled: false,
        hidden: false,
        comment: "Aime 读卡器模拟",
        fields: {
            pub cvt_port: u32 = 2,
            key: "cvtPort",
            comment: "CVT 模式串口号";
            pub sp_port: u32 = 3,
            key: "spPort",
            comment: "SP 模式串口号";
            pub high_baudrate: bool = false,
            key: "highBaud",
            comment: "使用高波特率";
            pub aime_path: String = String::from("aime.txt"),
            key: "aimePath",
            comment: "Aime 卡号文件";
            pub felica_path: String = String::from("felica.txt"),
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
            pub gen: u8 = 3,
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
        default_enabled: true,
        always_enabled: true,
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
}

impl AimeReader {
    pub fn new(config: AimeConfig) -> Self {
        Self { config }
    }

    pub fn read_aime_id(&self) -> Option<[u8; 10]> {
        if !is_scan_key_down(self.config.scan_key) {
            return None;
        }
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                return unsafe { external.read_aime_id() };
            }
        }
        read_or_generate_id(&self.config.aime_path, 10, self.config.aime_gen)
            .and_then(|bytes| bytes.try_into().ok())
    }

    pub fn ports(&self, is_sp: bool) -> u32 {
        if is_sp {
            self.config.sp_port
        } else {
            self.config.cvt_port
        }
    }

    pub fn read_felica_id(&self) -> Option<u64> {
        if !is_scan_key_down(self.config.scan_key) {
            return None;
        }
        if let Ok(guard) = EXTERNAL.lock() {
            if let Some(external) = guard.as_ref() {
                return unsafe { external.read_felica_id() };
            }
        }
        let bytes = read_or_generate_id(&self.config.felica_path, 8, self.config.felica_gen)?;
        Some(
            bytes
                .into_iter()
                .fold(0u64, |acc, byte| (acc << 8) | u64::from(byte)),
        )
    }

    pub fn read_mifare_uid(&self) -> Option<[u8; 4]> {
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

    let path = config
        .section::<AimeIoConfig>()
        .map_or_else(String::new, |config| dll_path(&config));
    if !path.is_empty() {
        match unsafe { ExternalAimeIo::load(&path) } {
            Ok(external) => {
                let version = external.api_version;
                if let Ok(mut guard) = EXTERNAL.lock() {
                    *guard = Some(external);
                }
                api.log_info(&format!(
                    "AimeIo: external IO DLL loaded: {path} (API {version:#06x})"
                ));
            }
            Err(err) => {
                api.log_warn(&format!(
                    "AimeIo: external IO DLL failed ({err}), falling back to built-in"
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
    if let Ok(mut device) = AIME_DEVICE.lock() {
        *device = Some(SgNfcDevice::new(SgNfcConfig {
            gen: cfg.gen,
            proxy_flag: cfg.proxy_flag,
            authdata_path: cfg.authdata_path.clone(),
        }));
    }
    unsafe {
        iohook::push_handler(aime_irp_handler);
    }
    api.log_info(&format!("Aime reader emulator initialized (COM{})", port));
}

unsafe fn aime_irp_handler(irp: &mut Irp) -> i32 {
    let port = aime_port();
    if irp.op == IrpOp::Open {
        if !matches_com_port(irp, port) {
            return iohook::invoke_next(irp);
        }
        uart::uart_init(port);
        let handle = uart::fake_handle_pub(port);
        let high_baudrate = AIME_READER
            .lock()
            .ok()
            .and_then(|reader| reader.as_ref().map(|reader| reader.config.high_baudrate))
            .unwrap_or(false);
        if !high_baudrate {
            uart::set_baud_rate(handle, 38_400);
        }
        irp.fd = handle as isize;
        AIME_FD.store(handle, Ordering::SeqCst);
        return 1;
    }

    let my_fd = AIME_FD.load(Ordering::SeqCst);
    if my_fd == 0 || irp.fd as usize != my_fd {
        return iohook::invoke_next(irp);
    }

    match irp.op {
        IrpOp::Close => {
            AIME_FD.store(0, Ordering::SeqCst);
            1
        }
        IrpOp::Read => read_aime(irp),
        IrpOp::Write => write_aime(irp),
        IrpOp::Ioctl => uart::device_io_control(
            irp.fd as usize,
            irp.ioctl,
            irp.ioctl_in,
            irp.ioctl_in_nbytes,
            irp.ioctl_out,
            irp.ioctl_out_nbytes,
            irp.out_nbytes,
        ),
        IrpOp::Fsync | IrpOp::Seek => 1,
        IrpOp::Open => 1,
    }
}

unsafe fn read_aime(irp: &mut Irp) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.read_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }
    let out = std::slice::from_raw_parts_mut(irp.read_buf, irp.nbytes as usize);
    let count = AIME_DEVICE
        .lock()
        .ok()
        .and_then(|mut device| device.as_mut().map(|device| device.read(out)))
        .unwrap_or(0);
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = count as u32;
    }
    1
}

unsafe fn write_aime(irp: &mut Irp) -> i32 {
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = 0;
    }
    if irp.write_buf.is_null() || irp.nbytes == 0 {
        return 1;
    }
    let bytes = std::slice::from_raw_parts(irp.write_buf, irp.nbytes as usize);
    if let (Ok(mut device), Ok(reader)) = (AIME_DEVICE.lock(), AIME_READER.lock()) {
        if let (Some(device), Some(reader)) = (device.as_mut(), reader.as_ref()) {
            device.write(reader, bytes);
        }
    }
    if !irp.out_nbytes.is_null() {
        *irp.out_nbytes = irp.nbytes;
    }
    1
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
