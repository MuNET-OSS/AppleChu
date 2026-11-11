use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::OnceLock;

use crate::util::api::Api;

// AdvertiseLogoObject::Impl 初始化时的第 0 阶段对应 AS_warning_in/out。
// 只修改 `mov [this+0x230], 0` 的立即数，使原生状态机从第 1 阶段
// AS_sega_in/out 开始，并继续执行后续全部场景转换。
const WARNING_STAGE_PATCH_RVA: usize = 0x893226;
const WARNING_STAGE_EXPECTED: u8 = 0;
const WARNING_STAGE_PATCHED: u8 = 1;

static API: OnceLock<Api> = OnceLock::new();
static WARNING_APPLIED: AtomicBool = AtomicBool::new(false);
static WARNING_PATCH_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static WARNING_PATCH_ORIGINAL: AtomicU8 = AtomicU8::new(0);

pub(crate) fn register_skip_warning(api: &Api, enabled: bool) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    let _ = API.set(*api);
    install_warning_patch(api)
}

pub(crate) fn shutdown() {
    if WARNING_APPLIED.swap(false, Ordering::AcqRel) {
        if let Some(api) = API.get() {
            let address = WARNING_PATCH_ADDRESS.load(Ordering::Acquire);
            let original = WARNING_PATCH_ORIGINAL.load(Ordering::Acquire);
            if address != 0 {
                let _ = api.mem_write(address, &[original]);
            }
        }
    }
    WARNING_PATCH_ADDRESS.store(0, Ordering::Release);
    WARNING_PATCH_ORIGINAL.store(0, Ordering::Release);
}

fn install_warning_patch(api: &Api) -> Result<(), String> {
    if WARNING_APPLIED.load(Ordering::Acquire) {
        return Ok(());
    }
    let address = api
        .game_base()
        .checked_add(WARNING_STAGE_PATCH_RVA)
        .ok_or_else(|| "warning stage patch address overflowed".to_owned())?;
    let mut current = [0u8; 1];
    if !api.mem_read(address, &mut current) {
        return Err(format!(
            "warning stage byte could not be read at 0x{address:08X}"
        ));
    }
    if current[0] == WARNING_STAGE_PATCHED {
        WARNING_PATCH_ADDRESS.store(address, Ordering::Release);
        WARNING_PATCH_ORIGINAL.store(WARNING_STAGE_EXPECTED, Ordering::Release);
    } else if current[0] == WARNING_STAGE_EXPECTED {
        if !api.mem_write(address, &[WARNING_STAGE_PATCHED]) {
            return Err(format!(
                "warning stage patch could not be written at 0x{address:08X}"
            ));
        }
        WARNING_PATCH_ADDRESS.store(address, Ordering::Release);
        WARNING_PATCH_ORIGINAL.store(current[0], Ordering::Release);
    } else {
        return Err(format!(
            "warning stage signature mismatch at 0x{address:08X}: found 0x{:02X}",
            current[0]
        ));
    }
    WARNING_APPLIED.store(true, Ordering::Release);
    api.log_info(&format!(
        "SkipWarning: Advertise starts at stage 1; warning stage skipped at 0x{address:08X}"
    ));
    Ok(())
}
