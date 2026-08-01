use std::cell::RefCell;

use super::*;

const BASE: usize = 0x1000;

struct FakeMemory {
    image: RefCell<Vec<u8>>,
}

impl FakeMemory {
    fn new(image: Vec<u8>) -> Self {
        Self {
            image: RefCell::new(image),
        }
    }
}

impl PatchMemory for FakeMemory {
    fn game_base(&self) -> usize {
        BASE
    }

    fn game_size(&self) -> u32 {
        self.image.borrow().len() as u32
    }

    fn aob_scan(&self, start: usize, size: u32, pattern: &[u8], mask: &str) -> usize {
        let offset = start - BASE;
        let image = self.image.borrow();
        find_pattern(&image[offset..offset + size as usize], pattern, mask)
            .map_or(0, |found| start + found)
    }

    fn mem_read(&self, addr: usize, buf: &mut [u8]) -> bool {
        let offset = addr - BASE;
        buf.copy_from_slice(&self.image.borrow()[offset..offset + buf.len()]);
        true
    }

    fn mem_write(&self, addr: usize, data: &[u8]) -> bool {
        let offset = addr - BASE;
        self.image.borrow_mut()[offset..offset + data.len()].copy_from_slice(data);
        true
    }

    fn log_info(&self, _message: &str) {}

    fn log_warn(&self, _message: &str) {}
}

fn config(source: &str) -> Config {
    Config::parse(".", source).expect("测试配置必须有效")
}

fn pe_image(size: usize) -> Vec<u8> {
    let mut image = vec![0x90; size];
    image[0..2].copy_from_slice(&0x5A4D_u16.to_le_bytes());
    image[0x3C..0x40].copy_from_slice(&0x80_i32.to_le_bytes());
    image[0x80..0x84].copy_from_slice(&0x0000_4550_u32.to_le_bytes());
    image[0x86..0x88].copy_from_slice(&1_u16.to_le_bytes());
    image[0x94..0x96].copy_from_slice(&0_u16.to_le_bytes());
    image[0xA0..0xA4].copy_from_slice(&(size as u32).to_le_bytes());
    image[0xA4..0xA8].copy_from_slice(&0_u32.to_le_bytes());
    image[0xA8..0xAC].copy_from_slice(&(size as u32).to_le_bytes());
    image[0xAC..0xB0].copy_from_slice(&0_u32.to_le_bytes());
    image
}

#[test]
fn early_scanner_matches_wildcards_before_tls() {
    // Given: 版本间变化的字节位于稳定指令签名中。
    let image = [0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x12, 0x80];
    let pattern = [0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x00, 0x80];

    // When: TLS 前扫描器使用与 patch 定义相同的掩码。
    let found = find_pattern(&image, &pattern, "xxxxxx?x");

    // Then: 可变字节不会破坏版本无关识别。
    assert_eq!(found, Some(0));
}

#[test]
fn shared_audio_is_patched_by_early_pipeline() {
    // Given: 无关结构初始化先出现旧短特征，真实共享模式参数位于后方。
    let mut image = vec![0x90; 96];
    image[8..18].copy_from_slice(&[0xC7, 0x86, 0xA0, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
    image[40..64].copy_from_slice(&[
        0x6A, 0x01, 0xE8, 0, 0, 0, 0, 0x8D, 0xBE, 0xD0, 0x00, 0x00, 0x00, 0x0F, 0x57, 0xC0, 0x0F,
        0x11, 0x07, 0xB8, 0xFE, 0xFF, 0x00, 0x00,
    ]);
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 early patch 管线。
    patches::apply_pre_tls(&memory, &config("[ForceSharedAudio]"));

    // Then: 只改写真实 WASAPI share mode，不触碰前面的同形指令。
    let image = memory.image.borrow();
    assert_eq!(image[14], 0x01);
    assert_eq!(image[41], 0x00);
}

#[test]
fn custom_timers_are_patched_by_early_pipeline() {
    // Given: 三类计时器指令保持默认立即数，配置给出不同值。
    let mut image = vec![0x90; 192];
    image[16..37].copy_from_slice(&[
        0x6A, 0x01, 0x8B, 0xCE, 0xE8, 0, 0, 0, 0, 0x68, 0x84, 0x03, 0, 0, 0x6A, 0x0A, 0x6A, 60,
        0x8B, 0xCE, 0xE8,
    ]);
    image[64..85].copy_from_slice(&[
        0x6A, 0x01, 0x8B, 0xCF, 0xE8, 0, 0, 0, 0, 0x68, 0x84, 0x03, 0, 0, 0x6A, 0x0A, 0x6A, 60,
        0x8B, 0xCF, 0xE8,
    ]);
    image[112..135].copy_from_slice(&[
        0xE8, 0, 0, 0, 0, 0x6A, 60, 0xE8, 0, 0, 0, 0, 0x83, 0xC4, 0x04, 0x8D, 0x4E, 0x08, 0x05,
        0x84, 0x03, 0, 0,
    ]);
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 读取三个配置值并执行 early patch。
    patches::apply_pre_tls(
        &memory,
        &config("[CustomTimers]\nmap_select=61\nticket_select=62\ncourse_select=63"),
    );

    // Then: map、ticket、course 各自的立即数均被正确改写。
    let image = memory.image.borrow();
    assert_eq!([image[81], image[33], image[118]], [61, 62, 63]);
}

#[test]
fn disable_timer_uses_unique_context_in_pre_tls_pipeline() {
    let mut image = vec![0x90; 160];
    image[8..11].copy_from_slice(&[0x32, 0xC0, 0xC3]);
    image[48..107].copy_from_slice(&[
        0x85, 0xC0, 0x74, 0x6F, 0x83, 0xF8, 0x08, 0x74, 0x6A, 0xE8, 0, 0, 0, 0, 0x3C, 0x01, 0x74,
        0x55, 0x8B, 0x0D, 0, 0, 0, 0, 0xE8, 0, 0, 0, 0, 0x8B, 0xC8, 0xE8, 0, 0, 0, 0, 0x8D, 0x48,
        0x78, 0xE8, 0, 0, 0, 0, 0x3C, 0x01, 0x74, 0x37, 0x56, 0x8D, 0x8F, 0xA8, 0, 0, 0, 0xE8, 0,
        0, 0,
    ]);
    let memory = FakeMemory::new(image);

    patches::apply_pre_tls(&memory, &config("[DisableTimer]"));

    let image = memory.image.borrow();
    assert_eq!(&image[8..11], &[0x32, 0xC0, 0xC3]);
    assert_eq!(image[94], 0xEB);
}

#[test]
fn max_tracks_is_patched_by_early_pipeline() {
    // Given: 无关函数先返回 3，真实曲数函数当前返回 11，配置要求 7。
    let mut image = vec![0x90; 96];
    image[8..14].copy_from_slice(&[0xB8, 0x03, 0, 0, 0, 0xC3]);
    image[40..72].copy_from_slice(&[
        0xB8, 0x0B, 0, 0, 0, 0xC3, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC,
        0x8B, 0x44, 0x24, 0x04, 0x53, 0xB3, 0x14, 0x8B, 0x10, 0x85, 0xD2, 0x78, 0x1E, 0x83, 0xFA,
        0x10,
    ]);
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 UnlockTracks early patch。
    patches::apply_pre_tls(&memory, &config("[UnlockTracks]\nmax=7"));

    // Then: 无关函数保持不变，真实曲数函数在入口点运行前从 11 改为 7。
    assert_eq!(&memory.image.borrow()[9..13], &3_i32.to_le_bytes());
    assert_eq!(&memory.image.borrow()[41..45], &7_i32.to_le_bytes());
}

#[test]
fn unlock_track_clamp_is_patched_for_247() {
    // Given: 247 clamp 指令仍使用 ESI 比较操作数。
    let offset = 0x6F8F92;
    let mut image = pe_image(offset + 3);
    image[offset..offset + 3].copy_from_slice(&[0xF0, 0x8B, 0xC6]);
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 247 UnlockTracks clamp 补丁。
    patches::apply_pre_tls(&memory, &config("[UnlockTracks]\nmax=7"));

    // Then: 247 的条件移动寄存器被改为自身，clamp 不再限制上限。
    assert_eq!(memory.image.borrow()[offset], 0xC0);
}

#[test]
fn unlock_track_clamp_is_patched_for_250() {
    // Given: 250 clamp 返回 7 与 ECX 的较小值。
    let offset = 0x3DF06D;
    let mut image = pe_image(offset + 11);
    image[offset..offset + 11]
        .copy_from_slice(&[0xB8, 0x07, 0, 0, 0, 0x3B, 0xC1, 0x0F, 0x47, 0xC1, 0xC3]);
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 250 UnlockTracks clamp 补丁。
    patches::apply_pre_tls(&memory, &config("[UnlockTracks]\nmax=7"));

    // Then: 250 返回上限改为 99，条件移动指令被 NOP。
    assert_eq!(
        &memory.image.borrow()[offset + 1..offset + 5],
        &99_i32.to_le_bytes()
    );
    assert_eq!(
        &memory.image.borrow()[offset + 7..offset + 10],
        &[0x90, 0x90, 0x90]
    );
}

#[test]
fn fast_restart_is_patched_by_early_pipeline() {
    // Given: D3D9Ex 快速重启的目标函数和失败分支仍为原始指令。
    let mut image = vec![0x90; 128];
    image[8..24].copy_from_slice(&[
        0xE8, 0x43, 0, 0, 0, 0x84, 0xC0, 0x74, 0xDB, 0x8D, 0x8B, 0x10, 0, 0, 0, 0xE8,
    ]);
    image[80..83].copy_from_slice(&[0x55, 0x8B, 0xEC]);
    image[96..102].copy_from_slice(&[0xC2, 0x83, 0xF8, 0x07, 0x74, 0x20]);
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 D3D9Ex early patch。
    patches::apply_pre_tls(&memory, &config("[D3D9Ex]\nfast_restart=true"));

    // Then: 目标函数与失败分支在游戏入口点前同时完成改写。
    let image = memory.image.borrow();
    assert_eq!(&image[80..83], &[0xB0, 0x01, 0xC3]);
    assert_eq!(image[100], 0xEB);
}

#[test]
fn custom_version_is_patched_by_early_pipeline() {
    // Given: 版本字符串包含稳定的 X-VERSE 后缀，配置提供自定义文字。
    let mut image = vec![0x90; 224];
    image[79] = 0;
    image[80..96].copy_from_slice(b"CHUNITHM X-VERSE");
    image[96] = 0;
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 General early patch。
    patches::apply_pre_tls(&memory, &config("[General]\nversion_text='EARLY'"));

    // Then: 自定义版本文字在游戏入口点前写入。
    assert_eq!(&memory.image.borrow()[80..86], b"EARLY\0");
}

#[test]
fn custom_free_play_text_is_patched_by_early_pipeline() {
    // Given: FREE PLAY 字符串和其 PUSH 长度调用点仍为默认值。
    let mut image = vec![0x90; 192];
    let text_addr = u32::try_from(BASE + 120).expect("测试地址必须适合 x86");
    image[24..31].copy_from_slice(&[
        0x6A,
        9,
        0x68,
        text_addr.to_le_bytes()[0],
        text_addr.to_le_bytes()[1],
        text_addr.to_le_bytes()[2],
        text_addr.to_le_bytes()[3],
    ]);
    image[120..130].copy_from_slice(b"FREE PLAY\0");
    let memory = FakeMemory::new(image);

    // When: DLL_PROCESS_ATTACH 执行 FreePlay early patch。
    patches::apply_pre_tls(&memory, &config("[FreePlay]\ncustom_text='EARLY'"));

    // Then: 字符串长度与内容在游戏入口点前保持一致。
    let image = memory.image.borrow();
    assert_eq!(image[25], 5);
    assert_eq!(&image[120..126], b"EARLY\0");
}

#[test]
fn tls_and_appuser_are_patched_in_single_pre_tls_pass() {
    // Given: AppUser 与 TLS 检测仍是原字节，且配置明确启用两项绕过。
    let mut image = vec![0x90; 64];
    image[8..14].copy_from_slice(&[0x83, 0x7C, 0x24, 0x04, 0x00, 0x75]);
    image[24..40].copy_from_slice(&[
        0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x00, 0x80, 0x00, 0xEB, 0x02, 0x33, 0xF6, 0x8B, 0x5B,
        0x34,
    ]);
    let memory = FakeMemory::new(image);

    let config = config("[BypassAppUser]\n[DisableTLS]");

    // When: DLL_PROCESS_ATTACH 在 EXE TLS callback 前执行全部内存补丁。
    patches::apply_pre_tls(&memory, &config);

    // Then: 两个可能在初始化中被缓存的检测都已提前改写。
    let image = memory.image.borrow();
    assert_eq!(image[13], 0xEB);
    assert_eq!(image[31], 0x00);
}

#[test]
fn invalid_config_skips_pre_tls_memory_changes() {
    // Given: TLS 特征存在，但配置版本无效。
    let mut image = vec![0x90; 48];
    image[16..32].copy_from_slice(&[
        0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x00, 0x80, 0x00, 0xEB, 0x02, 0x33, 0xF6, 0x8B, 0x5B,
        0x34,
    ]);
    let memory = FakeMemory::new(image);
    let config = Config::parse(".", "Version = \"0\"\n[DisableTLS]\n").expect("TOML 语法必须有效");

    // When: TLS 前入口尝试应用配置。
    apply_pre_tls_if_valid(&memory, &config);

    // Then: 校验失败会阻止所有 early patch 写入。
    assert_eq!(memory.image.borrow()[23], 0x80);
}

#[test]
fn appuser_uses_current_revision_site_in_single_pre_tls_pass() {
    // Given: 2.50 同时保留旧检查函数与当前实际检查函数。
    let mut image = pe_image(0x8B400);
    image[0x8A0C0..0x8A0E2].copy_from_slice(&[
        0x83, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x1A, 0x56, 0xE8, 0x13, 0xF4, 0xFF, 0xFF, 0x8B, 0x70,
        0x04, 0xE8, 0x9B, 0xFF, 0xFF, 0xFF, 0x56, 0x8B, 0xC8, 0xE8, 0x13, 0xFF, 0xFF, 0xFF, 0x83,
        0xC4, 0x04, 0x5E, 0xC3,
    ]);
    image[0x8B3A0..0x8B3C2].copy_from_slice(&[
        0x83, 0x7C, 0x24, 0x04, 0x00, 0x75, 0x1A, 0x56, 0xE8, 0x33, 0xE1, 0xFF, 0xFF, 0x8B, 0x70,
        0x04, 0xE8, 0x9B, 0xFF, 0xFF, 0xFF, 0x56, 0x8B, 0xC8, 0xE8, 0x13, 0xFF, 0xFF, 0xFF, 0x83,
        0xC4, 0x04, 0x5E, 0xC3,
    ]);
    let memory = FakeMemory::new(image);

    // When: AppUser patch 在 EXE TLS callback 前只执行一次。
    patches::apply_pre_tls(&memory, &config("[BypassAppUser]"));

    // Then: 旧函数保持原样，当前版本位点被绕过。
    let image = memory.image.borrow();
    assert_eq!(image[0x8A0C5], 0x75);
    assert_eq!(image[0x8B3A5], 0xEB);
}
