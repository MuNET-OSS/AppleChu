//! chu2to3 shared-memory producer (x86 side).
//!
//! Faithful port of segatools' chu2to3 engine, x86 half. When a single 32-bit
//! chuniio DLL is loaded via the `[ChuniIo] path` setting, the 64-bit
//! `chusanhook_x64` injected into `amdaemon.exe` runs the chu2to3 engine in its
//! x64 half, which only does `OpenFileMapping("Local\\Chu2to3Shmem")` and reads
//! JVS state from there. This module creates that shared memory and continuously
//! writes opbtn/beams/coin/version into it, exactly like segatools'
//! `jvs_poll_thread_proc`, so the amdaemon side comes alive.
//!
//! Layout reference (segatools games/chuniio/chu2to3.c):
//! ```c
//! #pragma pack(1)
//! typedef struct shared_data_s {
//!     uint16_t coin_counter;
//!     uint8_t  opbtn;
//!     uint8_t  beams;
//!     uint16_t version;
//! } shared_data_t;
//! TCHAR g_shmem_name[] = TEXT("Local\\Chu2to3Shmem");
//! #define BUF_SIZE 1024
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, FILE_MAP_ALL_ACCESS, PAGE_READWRITE,
};

use crate::util::api::Api;

use super::external::JvsRawFns;

const SHMEM_NAME: &[u16] = &[
    b'L' as u16, b'o' as u16, b'c' as u16, b'a' as u16, b'l' as u16, b'\\' as u16, b'C' as u16,
    b'h' as u16, b'u' as u16, b'2' as u16, b't' as u16, b'o' as u16, b'3' as u16, b'S' as u16,
    b'h' as u16, b'm' as u16, b'e' as u16, b'm' as u16, 0,
];
const BUF_SIZE: u32 = 1024;
const SHARED_DATA_LEN: usize = 6;

static STARTED: AtomicBool = AtomicBool::new(false);

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

/// Start the chu2to3 x86 producer for a single-DLL (`path`) external chuniio.
///
/// Creates `Local\Chu2to3Shmem`, writes the API version immediately (so the
/// x64 side's version handshake doesn't time out), runs `jvs_init`, then spawns
/// a 1 ms polling thread mirroring segatools' `jvs_poll_thread_proc`.
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
        api.log_warn("chu2to3: CreateFileMapping failed; amdaemon JVS bridge inactive");
        STARTED.store(false, Ordering::SeqCst);
        return;
    }

    let mapped = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, 0) };
    if mapped.Value.is_null() {
        api.log_warn("chu2to3: MapViewOfFile failed; amdaemon JVS bridge inactive");
        unsafe { CloseHandle(mapping) };
        STARTED.store(false, Ordering::SeqCst);
        return;
    }
    let view = ShmemView(mapped.Value as *mut u8);

    // Write the API version up front. The x64 side polls until version != 0 to
    // confirm the handshake, so writing it now avoids its 3x5s timeout.
    view.write(0, 0, 0, fns.api_version);

    unsafe { fns.jvs_init() };

    api.log_info(&format!(
        "chu2to3: shared memory bridge started (API {:#06x})",
        fns.api_version
    ));

    let mapping = crate::util::win32::handle_value(mapping);
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
}
