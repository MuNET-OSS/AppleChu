use std::ffi::{c_char, CStr};

static RTTI_RDATA_BASE: once_cell::sync::Lazy<std::sync::atomic::AtomicUsize> =
    once_cell::sync::Lazy::new(|| std::sync::atomic::AtomicUsize::new(0));
static RTTI_RDATA_SIZE: once_cell::sync::Lazy<std::sync::atomic::AtomicUsize> =
    once_cell::sync::Lazy::new(|| std::sync::atomic::AtomicUsize::new(0));
static RTTI_TEXT_BASE: once_cell::sync::Lazy<std::sync::atomic::AtomicUsize> =
    once_cell::sync::Lazy::new(|| std::sync::atomic::AtomicUsize::new(0));

pub fn set_rtti_info(rdata_base: usize, rdata_size: usize, text_base: usize) {
    RTTI_RDATA_BASE.store(rdata_base, std::sync::atomic::Ordering::Relaxed);
    RTTI_RDATA_SIZE.store(rdata_size, std::sync::atomic::Ordering::Relaxed);
    RTTI_TEXT_BASE.store(text_base, std::sync::atomic::Ordering::Relaxed);
}

unsafe fn find_pattern_u32(base: usize, size: usize, value: u32) -> Vec<usize> {
    let mut results = Vec::new();
    let needle = value.to_le_bytes();
    let data = std::slice::from_raw_parts(base as *const u8, size);
    let mut i = 0;
    while i + 4 <= data.len() {
        if data[i..i + 4] == needle {
            results.push(base + i);
        }
        i += 1;
    }
    results
}

pub unsafe extern "C" fn api_rtti_find_vtable(rtti_name: *const c_char) -> usize {
    if rtti_name.is_null() {
        return 0;
    }
    let name = CStr::from_ptr(rtti_name);
    let target = name.to_bytes();
    if target.is_empty() {
        return 0;
    }

    let rdata_base = RTTI_RDATA_BASE.load(std::sync::atomic::Ordering::Relaxed);
    let rdata_size = RTTI_RDATA_SIZE.load(std::sync::atomic::Ordering::Relaxed);
    let text_base = RTTI_TEXT_BASE.load(std::sync::atomic::Ordering::Relaxed);
    if rdata_base == 0 || rdata_size == 0 || text_base == 0 {
        return 0;
    }

    let rdata = std::slice::from_raw_parts(rdata_base as *const u8, rdata_size);
    let mut offset = 0;
    while offset + target.len() < rdata.len() {
        if &rdata[offset..offset + target.len()] == target
            && rdata.get(offset + target.len()) == Some(&0)
        {
            let name_va = rdata_base + offset;
            let td_va = name_va - 8;

            let pvf = *(td_va as *const u32);
            if (pvf as usize) < rdata_base {
                offset += 1;
                continue;
            }

            let refs = find_pattern_u32(rdata_base, rdata_size, td_va as u32);
            for ref_va in &refs {
                let col_va = ref_va - 0x0C;
                if col_va < rdata_base {
                    continue;
                }
                if *(col_va as *const u32) != 0 {
                    continue;
                }
                if *((col_va + 0x0C) as *const u32) as usize != td_va {
                    continue;
                }

                let col_refs = find_pattern_u32(rdata_base, rdata_size, col_va as u32);
                for col_ref in &col_refs {
                    let vtable_va = col_ref + 4;
                    let first_entry = *(vtable_va as *const u32);
                    if (first_entry as usize) >= text_base {
                        return vtable_va;
                    }
                }
            }
        }
        offset += 1;
    }
    0
}
