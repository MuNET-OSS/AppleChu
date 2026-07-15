use std::mem::size_of;

use crate::util::api::Api;

pub trait PatchMemory {
    fn game_base(&self) -> usize;
    fn game_size(&self) -> u32;
    fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize;
    fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool;
    fn mem_write(&self, addr: usize, data: &[u8]) -> bool;
    fn log_info(&self, message: &str);
    fn log_warn(&self, message: &str);
}

impl PatchMemory for Api {
    fn game_base(&self) -> usize {
        Api::game_base(self)
    }

    fn game_size(&self) -> u32 {
        Api::game_size(self)
    }

    fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize {
        Api::aob_scan(self, start, size, pattern, mask)
    }

    fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool {
        Api::mem_read(self, addr, buf)
    }

    fn mem_write(&self, addr: usize, data: &[u8]) -> bool {
        Api::mem_write(self, addr, data)
    }

    fn log_info(&self, message: &str) {
        Api::log_info(self, message);
    }

    fn log_warn(&self, message: &str) {
        Api::log_warn(self, message);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchResult {
    Applied,
    AlreadyPatched,
    Mismatch,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ImageDosHeader {
    e_magic: u16,
    _unused: [u8; 58],
    e_lfanew: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ImageFileHeader {
    _machine: u16,
    number_of_sections: u16,
    _time_date_stamp: u32,
    _pointer_to_symbol_table: u32,
    _number_of_symbols: u32,
    size_of_optional_header: u16,
    _characteristics: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ImageSectionHeader {
    _name: [u8; 8],
    virtual_size: u32,
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
    _unused: [u8; 16],
}

const IMAGE_DOS_SIGNATURE: u16 = 0x5A4D;
const IMAGE_NT_SIGNATURE: u32 = 0x0000_4550;

pub fn file_offset_to_va<M: PatchMemory>(api: &M, file_offset: u32) -> usize {
    let base = api.game_base();
    if base == 0 {
        return 0;
    }

    let Some(dos) = read_struct::<ImageDosHeader, _>(api, base) else {
        return 0;
    };
    if dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 {
        return 0;
    }

    let nt_addr = base + dos.e_lfanew as usize;
    let Some(signature) = read_struct::<u32, _>(api, nt_addr) else {
        return 0;
    };
    if signature != IMAGE_NT_SIGNATURE {
        return 0;
    }

    let file_header_addr = nt_addr + size_of::<u32>();
    let Some(file_header) = read_struct::<ImageFileHeader, _>(api, file_header_addr) else {
        return 0;
    };

    let section_addr = file_header_addr
        + size_of::<ImageFileHeader>()
        + file_header.size_of_optional_header as usize;
    for index in 0..file_header.number_of_sections as usize {
        let Some(section) = read_struct::<ImageSectionHeader, _>(
            api,
            section_addr + index * size_of::<ImageSectionHeader>(),
        ) else {
            return 0;
        };

        let raw_start = section.pointer_to_raw_data;
        let raw_size = section.size_of_raw_data.max(section.virtual_size);
        let raw_end = raw_start.saturating_add(raw_size);
        if (raw_start..raw_end).contains(&file_offset) {
            let rva = file_offset - raw_start + section.virtual_address;
            return base + rva as usize;
        }
    }

    0
}

pub fn patch_bytes<M: PatchMemory>(
    api: &M,
    addr: usize,
    expected: &[u8],
    patch: &[u8],
) -> PatchResult {
    if addr == 0 || expected.len() != patch.len() {
        return PatchResult::Mismatch;
    }

    let mut current = vec![0; expected.len()];
    if !api.mem_read(addr, &mut current) {
        return PatchResult::Mismatch;
    }

    if current == patch {
        return PatchResult::AlreadyPatched;
    }

    if current != expected {
        return PatchResult::Mismatch;
    }

    if api.mem_write(addr, patch) {
        PatchResult::Applied
    } else {
        PatchResult::Mismatch
    }
}

fn read_struct<T: Copy, M: PatchMemory>(api: &M, addr: usize) -> Option<T> {
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    // SAFETY: Category 4（未初始化内存）。切片覆盖 T 的完整对象大小，仅交给 mem_read
    // 作为输出缓冲区；在 mem_read 成功前不会读取其中任何字节。
    let bytes =
        unsafe { std::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>()) };
    if !api.mem_read(addr, bytes) {
        return None;
    }
    // SAFETY: Category 4（未初始化内存）。mem_read 的成功契约表示完整缓冲区均已写入，
    // 且本函数仅用于读取由 #[repr(C)]/整数构成的 PE 结构。
    Some(unsafe { value.assume_init() })
}
