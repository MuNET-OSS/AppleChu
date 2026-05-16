use std::mem::size_of;

use crate::util::api::Api;

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

pub fn file_offset_to_va(api: &Api, file_offset: u32) -> usize {
    let base = api.game_base();
    if base == 0 {
        return 0;
    }

    let Some(dos) = read_struct::<ImageDosHeader>(api, base) else {
        return 0;
    };
    if dos.e_magic != IMAGE_DOS_SIGNATURE || dos.e_lfanew < 0 {
        return 0;
    }

    let nt_addr = base + dos.e_lfanew as usize;
    let Some(signature) = read_struct::<u32>(api, nt_addr) else {
        return 0;
    };
    if signature != IMAGE_NT_SIGNATURE {
        return 0;
    }

    let file_header_addr = nt_addr + size_of::<u32>();
    let Some(file_header) = read_struct::<ImageFileHeader>(api, file_header_addr) else {
        return 0;
    };

    let section_addr = file_header_addr + size_of::<ImageFileHeader>() + file_header.size_of_optional_header as usize;
    for index in 0..file_header.number_of_sections as usize {
        let Some(section) = read_struct::<ImageSectionHeader>(
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

pub fn patch_bytes(api: &Api, addr: usize, expected: &[u8], patch: &[u8]) -> PatchResult {
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

pub fn nop_bytes(api: &Api, addr: usize, expected: &[u8], count: usize) -> PatchResult {
    patch_bytes(api, addr, expected, &vec![0x90; count])
}

pub fn write_value<T>(api: &Api, addr: usize, value: T) -> bool {
    let bytes = unsafe { std::slice::from_raw_parts((&value as *const T).cast::<u8>(), size_of::<T>()) };
    api.mem_write(addr, bytes)
}

fn read_struct<T: Copy>(api: &Api, addr: usize) -> Option<T> {
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let bytes = unsafe { std::slice::from_raw_parts_mut(value.as_mut_ptr().cast::<u8>(), size_of::<T>()) };
    api.mem_read(addr, bytes).then(|| unsafe { value.assume_init() })
}
