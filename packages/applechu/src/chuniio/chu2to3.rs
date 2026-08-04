//! chu2to3 共享内存桥
//! x86 游戏侧写入 JVS 状态，x64 AM Daemon 侧按固定布局读取

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_ALL_ACCESS, PAGE_READWRITE,
};

use crate::util::api::Api;

use super::external::JvsRawFns;

const SHMEM_NAME: &[u16] = &[
    b'L' as u16,
    b'o' as u16,
    b'c' as u16,
    b'a' as u16,
    b'l' as u16,
    b'\\' as u16,
    b'C' as u16,
    b'h' as u16,
    b'u' as u16,
    b'2' as u16,
    b't' as u16,
    b'o' as u16,
    b'3' as u16,
    b'S' as u16,
    b'h' as u16,
    b'm' as u16,
    b'e' as u16,
    b'm' as u16,
    0,
];
const BUF_SIZE: u32 = 1024;
const SHARED_DATA_LEN: usize = 6;

static STARTED: AtomicBool = AtomicBool::new(false);
static PRODUCER_INITIALIZED: AtomicBool = AtomicBool::new(false);
static PRODUCER_FNS: Mutex<Option<JvsRawFns>> = Mutex::new(None);
static MAPPING_HANDLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static MAPPING_VIEW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(windows, target_pointer_width = "64"))]
use std::sync::OnceLock;

#[cfg(all(windows, target_pointer_width = "64"))]
use windows_sys::Win32::System::Memory::{OpenFileMappingW, FILE_MAP_READ};

#[cfg(all(windows, target_pointer_width = "64"))]
static CONSUMER: OnceLock<SharedMemoryConsumer> = OnceLock::new();

#[cfg(all(windows, target_pointer_width = "64"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SharedData {
    pub coin_counter: u16,
    pub opbtn: u8,
    pub beams: u8,
    version: u16,
}

#[cfg(all(windows, target_pointer_width = "64"))]
impl SharedData {
    fn decode(bytes: [u8; SHARED_DATA_LEN]) -> Self {
        Self {
            coin_counter: u16::from_le_bytes([bytes[0], bytes[1]]),
            opbtn: bytes[2],
            beams: bytes[3],
            version: u16::from_le_bytes([bytes[4], bytes[5]]),
        }
    }
}

#[cfg(all(windows, target_pointer_width = "64"))]
struct SharedMemoryConsumer {
    _mapping: windows_sys::Win32::Foundation::HANDLE,
    view: *const u8,
}

#[cfg(all(windows, target_pointer_width = "64"))]
// SAFETY: 映射在进程生命周期内保持有效，且只经易失读取访问；不会借出 Rust 引用
unsafe impl Send for SharedMemoryConsumer {}

#[cfg(all(windows, target_pointer_width = "64"))]
// SAFETY: 对共享映射的读取不修改 Rust 管理的内存，跨线程读取与外部进程写入由 Windows 映射协议协调
unsafe impl Sync for SharedMemoryConsumer {}

#[cfg(all(windows, target_pointer_width = "64"))]
impl SharedMemoryConsumer {
    fn open() -> Result<Self, &'static str> {
        // SAFETY: 名称以 NUL 结尾，指向静态 UTF-16 缓冲区
        let mapping = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, SHMEM_NAME.as_ptr()) };
        if mapping.is_null() {
            return Err("OpenFileMappingW failed");
        }
        // SAFETY: `mapping` 是有效文件映射句柄，读取范围由 Windows 映射对象保证
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, BUF_SIZE as usize) };
        if view.Value.is_null() {
            // SAFETY: `mapping` 由本函数成功创建，尚未移交所有权
            unsafe { CloseHandle(mapping) };
            return Err("MapViewOfFile failed");
        }
        Ok(Self {
            _mapping: mapping,
            view: view.Value.cast(),
        })
    }

    fn read(&self) -> SharedData {
        let mut bytes = [0_u8; SHARED_DATA_LEN];
        for (index, byte) in bytes.iter_mut().enumerate() {
            // SAFETY: 映射至少为 `BUF_SIZE` 字节，索引严格小于 `SHARED_DATA_LEN`
            *byte = unsafe { self.view.add(index).read_volatile() };
        }
        SharedData::decode(bytes)
    }
}

#[cfg(all(windows, target_pointer_width = "64"))]
pub(super) fn connect() -> Result<u16, &'static str> {
    if let Some(consumer) = CONSUMER.get() {
        return Ok(wait_for_version(consumer));
    }

    for attempt in 0..=10 {
        if let Ok(consumer) = SharedMemoryConsumer::open() {
            if CONSUMER.set(consumer).is_err() && CONSUMER.get().is_none() {
                return Err("shared memory initialization failed");
            }
            return CONSUMER
                .get()
                .map(wait_for_version)
                .ok_or("shared memory initialization failed");
        }
        if attempt != 10 {
            thread::sleep(Duration::from_secs(5));
        }
    }
    Err("Chu2to3 shared memory was not created by the x86 process")
}

#[cfg(all(windows, target_pointer_width = "64"))]
fn wait_for_version(consumer: &SharedMemoryConsumer) -> u16 {
    for attempt in 0..=3 {
        let version = consumer.read().version;
        if version != 0 || attempt == 3 {
            return version.max(0x0100);
        }
        thread::sleep(Duration::from_secs(5));
    }
    0x0100
}

#[cfg(all(windows, target_pointer_width = "64"))]
pub(super) fn active() -> bool {
    CONSUMER.get().is_some()
}

#[cfg(all(windows, target_pointer_width = "64"))]
pub(super) fn read() -> Option<SharedData> {
    CONSUMER.get().map(SharedMemoryConsumer::read)
}

#[cfg(all(windows, target_pointer_width = "64"))]
pub(super) fn api_version() -> Option<u16> {
    CONSUMER.get().map(wait_for_version)
}

struct ShmemView(*mut u8);

unsafe impl Send for ShmemView {}

impl ShmemView {
    fn write(&self, coin: u16, opbtn: u8, beams: u8, version: u16) {
        let mut buf = [0u8; SHARED_DATA_LEN];
        buf[0..2].copy_from_slice(&coin.to_le_bytes());
        buf[2] = opbtn;
        buf[3] = beams;
        buf[4..6].copy_from_slice(&version.to_le_bytes());
        unsafe {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), self.0, SHARED_DATA_LEN);
        }
    }
}

/// 创建共享映射并在轮询线程启动前发布 API 版本
pub fn start(api: &Api, fns: JvsRawFns) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    let mapping = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE,
            0,
            BUF_SIZE,
            SHMEM_NAME.as_ptr(),
        )
    };
    if mapping.is_null() {
        api.log_warn("Failed to create chu2to3 shared memory; AM Daemon JVS bridge is disabled");
        STARTED.store(false, Ordering::SeqCst);
        return;
    }

    let mapped = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 0) };
    if mapped.Value.is_null() {
        api.log_warn("Failed to map chu2to3 shared memory; AM Daemon JVS bridge is disabled");
        unsafe { CloseHandle(mapping) };
        STARTED.store(false, Ordering::SeqCst);
        return;
    }
    let view = ShmemView(mapped.Value as *mut u8);
    MAPPING_HANDLE.store(crate::util::win32::handle_value(mapping), Ordering::Release);
    MAPPING_VIEW.store(mapped.Value as usize, Ordering::Release);

    // x64 侧等待非零版本号，必须在后端初始化前完成握手
    view.write(0, 0, 0, fns.api_version);

    if let Ok(mut stored) = PRODUCER_FNS.lock() {
        *stored = Some(fns);
    }

    api.log_info(&format!(
        "chu2to3: shared memory bridge started (API {:#06x})",
        fns.api_version
    ));
}

/// x86 侧由 IO4 初始化调用；x64 侧没有生产者，直接返回成功
pub fn producer_jvs_init() -> i32 {
    if !STARTED.load(Ordering::Acquire) {
        return -1;
    }
    if PRODUCER_INITIALIZED.swap(true, Ordering::AcqRel) {
        return 0;
    }
    let Some(fns) = PRODUCER_FNS.lock().ok().and_then(|mut fns| fns.take()) else {
        PRODUCER_INITIALIZED.store(false, Ordering::Release);
        return -1;
    };
    let status = unsafe { fns.jvs_init() };
    if status < 0 {
        PRODUCER_INITIALIZED.store(false, Ordering::Release);
        return status;
    }

    let mapping = MAPPING_HANDLE.load(Ordering::Acquire);
    let view = MAPPING_VIEW.load(Ordering::Acquire) as *mut u8;
    if view.is_null() {
        return -1;
    }
    let view = ShmemView(view);
    thread::spawn(move || {
        let _keep_mapping = mapping;
        loop {
            let coin = unsafe { fns.jvs_read_coin() };
            let mut opbtn = 0u8;
            let mut beams = 0u8;
            unsafe { fns.jvs_poll(&mut opbtn, &mut beams) };
            view.write(coin, opbtn, beams, fns.api_version);
            thread::sleep(Duration::from_millis(1));
        }
    });
    0
}

#[cfg(all(windows, target_pointer_width = "32"))]
pub fn producer_active() -> bool {
    STARTED.load(Ordering::Acquire)
}

#[cfg(all(test, windows, target_pointer_width = "64"))]
mod consumer_tests {
    use super::*;

    #[test]
    fn shared_memory_layout_matches_chu2to3_protocol() {
        assert_eq!(
            SharedData::decode([0x34, 0x12, 0x05, 0x3F, 0x02, 0x01]),
            SharedData {
                coin_counter: 0x1234,
                opbtn: 0x05,
                beams: 0x3F,
                version: 0x0102,
            }
        );
    }
}
