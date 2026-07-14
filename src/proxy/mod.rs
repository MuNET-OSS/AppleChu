#![allow(
    non_snake_case,
    clippy::manual_c_str_literals,
    clippy::upper_case_acronyms
)]

mod api_impl;
mod d3d9;
mod entry_pe;
mod loader;
mod x86_decoder;

use std::ffi::c_void;
use std::ptr;

use windows_sys_loader::Win32::Foundation::{BOOL, HMODULE, TRUE};
use windows_sys_loader::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys_loader::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, GetModuleHandleA,
};
use windows_sys_loader::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
};
use windows_sys_loader::Win32::System::Threading::GetCurrentProcess;

const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_PROCESS_DETACH: u32 = 0;
const JMP_REL32_LEN: usize = 5;
const HIJACK_ORIGINAL_LEN: usize = 16;
const ENTRY_STUB_LEN: usize = 16;
const TRAMPOLINE_EXTRA_LEN: usize = 5;

static mut HIJACK: EntryHijack = EntryHijack::new();

struct EntryHijack {
    game_base: usize,
    entry: *mut u8,
    overwritten_len: usize,
    original: [u8; HIJACK_ORIGINAL_LEN],
    trampoline: *mut u8,
    stub: *mut u8,
    installed: bool,
}

impl EntryHijack {
    const fn new() -> Self {
        Self {
            game_base: 0,
            entry: ptr::null_mut(),
            overwritten_len: 0,
            original: [0; 16],
            trampoline: ptr::null_mut(),
            stub: ptr::null_mut(),
            installed: false,
        }
    }
}

#[no_mangle]
unsafe extern "system" fn DllMain(h_module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    match reason {
        DLL_PROCESS_ATTACH => {
            DisableThreadLibraryCalls(h_module);
            install_entry_hijack();
            crate::early_patch::apply(HIJACK.game_base);
        }
        DLL_PROCESS_DETACH if _reserved.is_null() => {
            loader::unload_mods();
        }
        _ => {}
    }
    TRUE
}

pub(crate) fn base_dir() -> Option<String> {
    loader::pe::get_self_base_dir()
}

pub(crate) unsafe fn game_size(game_base: usize) -> u32 {
    loader::pe::parse_game_info(game_base as *mut c_void).0
}

unsafe fn install_entry_hijack() {
    if HIJACK.installed {
        return;
    }

    let game = GetModuleHandleA(ptr::null());
    if game.is_null() {
        diag("hijack: GetModuleHandleA(NULL) returned null");
        return;
    }
    let game_base = game as usize;
    let Some(entry) = entry_pe::image_entry_point(game_base) else {
        diag("hijack: image_entry_point failed");
        return;
    };
    let Some(overwritten_len) = x86_decoder::instruction_span(entry, JMP_REL32_LEN) else {
        diag(&format!("hijack: instruction_span failed at {:p}", entry));
        return;
    };
    if overwritten_len > HIJACK_ORIGINAL_LEN {
        diag(&format!(
            "hijack: overwritten_len {} too large",
            overwritten_len
        ));
        return;
    }

    ptr::copy_nonoverlapping(
        entry,
        ptr::addr_of_mut!(HIJACK.original).cast::<u8>(),
        overwritten_len,
    );
    let Some(trampoline) = build_trampoline(entry, overwritten_len) else {
        return;
    };
    let Some(stub) = build_entry_stub(entry) else {
        return;
    };

    if !write_jmp(entry, stub) {
        return;
    }

    HIJACK.game_base = game_base;
    HIJACK.entry = entry;
    HIJACK.overwritten_len = overwritten_len;
    HIJACK.trampoline = trampoline;
    HIJACK.stub = stub;
    HIJACK.installed = true;
    diag(&format!(
        "hijack: installed, entry={:p}, overwritten={}, stub={:p}, trampoline={:p}",
        entry, overwritten_len, stub, trampoline
    ));
}

unsafe fn build_trampoline(entry: *mut u8, overwritten_len: usize) -> Option<*mut u8> {
    let size = overwritten_len + TRAMPOLINE_EXTRA_LEN;
    let trampoline = VirtualAlloc(
        ptr::null_mut(),
        size,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_EXECUTE_READWRITE,
    ) as *mut u8;
    if trampoline.is_null() {
        return None;
    }

    ptr::copy_nonoverlapping(entry, trampoline, overwritten_len);
    write_rel32_jmp_unprotected(trampoline.add(overwritten_len), entry.add(overwritten_len));
    flush_icache(trampoline.cast(), size);
    Some(trampoline)
}

unsafe fn build_entry_stub(entry: *mut u8) -> Option<*mut u8> {
    let stub = VirtualAlloc(
        ptr::null_mut(),
        ENTRY_STUB_LEN,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_EXECUTE_READWRITE,
    ) as *mut u8;
    if stub.is_null() {
        return None;
    }

    let mut cursor = stub;
    *cursor = 0x60;
    cursor = cursor.add(1);
    *cursor = 0x9C;
    cursor = cursor.add(1);
    write_rel32_call(cursor, entry_bootstrap as *const () as *const u8);
    cursor = cursor.add(5);
    *cursor = 0x9D;
    cursor = cursor.add(1);
    *cursor = 0x61;
    cursor = cursor.add(1);
    write_rel32_jmp_unprotected(cursor, entry);
    flush_icache(stub.cast(), ENTRY_STUB_LEN);
    Some(stub)
}

unsafe extern "system" fn entry_bootstrap() {
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("bootstrap_diag.log")
        {
            let installed = ptr::addr_of!(HIJACK.installed).read();
            let _ = writeln!(f, "entry_bootstrap called, hijack.installed={}", installed);
        }
    }

    if HIJACK.installed {
        restore_entry_point();
    }

    loader::crash_dump::install();
    loader::load_mods();

    if HIJACK.game_base != 0 {
        d3d9::install_early(HIJACK.game_base);
    }
}

unsafe fn restore_entry_point() {
    let entry = HIJACK.entry;
    let len = HIJACK.overwritten_len;
    if entry.is_null() || len == 0 {
        return;
    }

    let mut old_protect = 0;
    if VirtualProtect(entry.cast(), len, PAGE_EXECUTE_READWRITE, &mut old_protect) == 0 {
        return;
    }
    ptr::copy_nonoverlapping(ptr::addr_of!(HIJACK.original).cast::<u8>(), entry, len);
    let mut ignored = 0;
    let _ = VirtualProtect(entry.cast(), len, old_protect, &mut ignored);
    flush_icache(entry.cast(), len);
}

unsafe fn write_jmp(address: *mut u8, target: *const u8) -> bool {
    let mut old_protect = 0;
    if VirtualProtect(
        address.cast(),
        JMP_REL32_LEN,
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return false;
    }
    write_rel32_jmp_unprotected(address, target);
    let mut ignored = 0;
    let _ = VirtualProtect(address.cast(), JMP_REL32_LEN, old_protect, &mut ignored);
    flush_icache(address.cast(), JMP_REL32_LEN);
    true
}

unsafe fn write_rel32_call(address: *mut u8, target: *const u8) {
    *address = 0xE8;
    write_rel32(address.add(1), address.add(5), target);
}

unsafe fn write_rel32_jmp_unprotected(address: *mut u8, target: *const u8) {
    *address = 0xE9;
    write_rel32(address.add(1), address.add(5), target);
}

unsafe fn write_rel32(slot: *mut u8, next_instruction: *const u8, target: *const u8) {
    let rel = (target as isize).wrapping_sub(next_instruction as isize) as i32;
    ptr::write_unaligned(slot.cast::<i32>(), rel);
}

unsafe fn flush_icache(address: *const c_void, len: usize) {
    let _ = FlushInstructionCache(GetCurrentProcess(), address, len);
}

fn diag(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("hijack_diag.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}
