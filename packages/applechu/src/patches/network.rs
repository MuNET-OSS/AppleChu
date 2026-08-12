use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
use crate::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use crate::util::api::Api;
use crate::util::iat_hook::hook_iat;
use crate::util::memory::PatchMemory;

const WINHTTP_DLL: &str = "winhttp.dll";
const WINHTTP_OPEN_REQUEST: &str = "WinHttpOpenRequest";
const WINHTTP_FLAG_SECURE: u32 = 0x0080_0000;
const TLS_FLAG_PATTERN: &str = "85 C0 75 07 BE 00 00 ?? 00 EB 02 33 F6 8B 5B 34";
const TLS_FLAG_PATTERN_OFFSET: isize = 7;
const ENCRYPTION_FIRST_PATTERN_OFFSET: isize = 0;
const ENCRYPTION_SECOND_PATTERN_OFFSET: isize = 4;
const ENCRYPTION_FIRST_F5_PATTERN: &str = concat!(
    "F5 00 00 00 ?? 00 00 00 ",
    "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ",
    "01 00 00 00 00 00 00 40 FF FF FF 3F"
);
const ENCRYPTION_SECOND_F5_PATTERN: &str = concat!(
    "?? 00 00 00 F5 00 00 00 ",
    "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ",
    "01 00 00 00 00 00 00 40 FF FF FF 3F"
);
const ENCRYPTION_FIRST_FA_PATTERN: &str = concat!(
    "FA 00 00 00 ?? 00 00 00 ",
    "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ",
    "01 00 00 00 00 00 00 40 FF FF FF 3F"
);
const ENCRYPTION_SECOND_FA_PATTERN: &str = concat!(
    "?? 00 00 00 FA 00 00 00 ",
    "?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ?? ",
    "01 00 00 00 00 00 00 40 FF FF FF 3F"
);

type WinHttpOpenRequestFn = unsafe extern "system" fn(
    *mut c_void,
    *const u16,
    *const u16,
    *const u16,
    *const u16,
    *const *const u16,
    u32,
) -> *mut c_void;

static ORIG_OPEN_REQUEST: AtomicUsize = AtomicUsize::new(0);

crate::config_section! {
    pub(crate) struct DisableEncryptionConfig => DISABLE_ENCRYPTION_CONFIG_SECTION {
        section: "DisableEncryption",
        order: 240,
        default_on: true,
        always_enabled: false,
        hidden: false,
        group: "network",
        comment: "关闭网络加密，私服需要",
        fields: {}
    }
}

crate::config_section! {
    pub(crate) struct DisableTlsConfig => DISABLE_TLS_CONFIG_SECTION {
        section: "DisableTLS",
        order: 250,
        default_on: true,
        always_enabled: false,
        hidden: false,
        group: "network",
        comment: "关闭 TLS，私服需要",
        fields: {}
    }
}

pub fn install_pre_entry_hook(api: &Api, config: &Config) {
    apply_disable_tls(api, config);
}

pub(crate) fn apply_early<M: PatchMemory>(api: &M, config: &Config) {
    if config
        .section::<DisableEncryptionConfig>()
        .is_some_and(|config| config.enabled)
    {
        apply_patch(
            api,
            &VersionedPatch {
                name: "disable encryption 1",
                variants: &[
                    PatchVariant {
                        pattern: Some(ENCRYPTION_FIRST_F5_PATTERN),
                        pattern_offset: ENCRYPTION_FIRST_PATTERN_OFFSET,
                        known_offsets: &[0x17D200C],
                        expected: &[0xF5],
                        patch: &[0x00],
                    },
                    PatchVariant {
                        pattern: Some(ENCRYPTION_FIRST_FA_PATTERN),
                        pattern_offset: ENCRYPTION_FIRST_PATTERN_OFFSET,
                        known_offsets: &[0x1812814],
                        expected: &[0xFA],
                        patch: &[0x00],
                    },
                ],
            },
        );
        apply_patch(
            api,
            &VersionedPatch {
                name: "disable encryption 2",
                variants: &[
                    PatchVariant {
                        pattern: Some(ENCRYPTION_SECOND_F5_PATTERN),
                        pattern_offset: ENCRYPTION_SECOND_PATTERN_OFFSET,
                        known_offsets: &[0x17D2010],
                        expected: &[0xF5],
                        patch: &[0x00],
                    },
                    PatchVariant {
                        pattern: Some(ENCRYPTION_SECOND_FA_PATTERN),
                        pattern_offset: ENCRYPTION_SECOND_PATTERN_OFFSET,
                        known_offsets: &[0x1812818],
                        expected: &[0xFA],
                        patch: &[0x00],
                    },
                ],
            },
        );
    }
    if config
        .section::<DisableTlsConfig>()
        .is_some_and(|config| config.enabled)
    {
        apply_patch(
            api,
            &VersionedPatch {
                name: "disable TLS flag",
                variants: &[PatchVariant {
                    pattern: Some(TLS_FLAG_PATTERN),
                    pattern_offset: TLS_FLAG_PATTERN_OFFSET,
                    known_offsets: &[0xE426CB],
                    expected: &[0x80],
                    patch: &[0x00],
                }],
            },
        );
    }
}

fn apply_disable_tls(api: &Api, config: &Config) {
    if !config
        .section::<DisableTlsConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }

    let original = unsafe {
        hook_iat(
            api.game_base(),
            WINHTTP_DLL,
            WINHTTP_OPEN_REQUEST,
            hooked_open_request as *const (),
        )
    };

    if let Some(orig) = original {
        ORIG_OPEN_REQUEST.store(orig as usize, Ordering::SeqCst);
        api.log_info("patch applied: disable TLS (WinHttpOpenRequest IAT hook)");
    } else {
        api.log_warn("disable TLS: WinHttpOpenRequest import not found");
    }
}

unsafe extern "system" fn hooked_open_request(
    h_connect: *mut c_void,
    verb: *const u16,
    object_name: *const u16,
    version: *const u16,
    referrer: *const u16,
    accept_types: *const *const u16,
    flags: u32,
) -> *mut c_void {
    let orig_addr = ORIG_OPEN_REQUEST.load(Ordering::SeqCst);
    if orig_addr == 0 {
        return std::ptr::null_mut();
    }

    let orig: WinHttpOpenRequestFn = std::mem::transmute(orig_addr);
    orig(
        h_connect,
        verb,
        object_name,
        version,
        referrer,
        accept_types,
        flags & !WINHTTP_FLAG_SECURE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_signature_matches_working_patch_when_revision_moves() {
        // Given: bytes around the secure-flag instruction in the supplied revision.
        let bytes = [
            0x85, 0xC0, 0x75, 0x07, 0xBE, 0x00, 0x00, 0x80, 0x00, 0xEB, 0x02, 0x33, 0xF6, 0x8B,
            0x5B, 0x34,
        ];

        // When: the version-independent TLS signature is located.
        let found = find_pattern(&bytes, TLS_FLAG_PATTERN);

        // Then: it selects the byte that the working executable clears.
        assert_eq!(
            found.and_then(|offset| offset.checked_add_signed(TLS_FLAG_PATTERN_OFFSET)),
            Some(7)
        );
        assert_eq!(bytes[7], 0x80);
    }

    #[test]
    fn encryption_signatures_match_fa_revision_when_offsets_move() {
        // Given: the encryption data block from the supplied revision.
        let bytes = encryption_fixture(0xFA);

        // When: both FA signatures are located independently.
        let first = find_pattern(&bytes, ENCRYPTION_FIRST_FA_PATTERN);
        let second = find_pattern(&bytes, ENCRYPTION_SECOND_FA_PATTERN);

        // Then: they select the two bytes cleared by the working executable.
        assert_eq!(
            first.and_then(|offset| offset.checked_add_signed(ENCRYPTION_FIRST_PATTERN_OFFSET)),
            Some(0)
        );
        assert_eq!(
            second.and_then(|offset| offset.checked_add_signed(ENCRYPTION_SECOND_PATTERN_OFFSET)),
            Some(4)
        );
    }

    #[test]
    fn encryption_signatures_keep_f5_revision_compatible() {
        // Given: the same data layout with the older revision's F5 values.
        let bytes = encryption_fixture(0xF5);

        // When: both legacy signatures are located independently.
        let first = find_pattern(&bytes, ENCRYPTION_FIRST_F5_PATTERN);
        let second = find_pattern(&bytes, ENCRYPTION_SECOND_F5_PATTERN);

        // Then: both legacy patch bytes remain discoverable.
        assert_eq!(
            first.and_then(|offset| offset.checked_add_signed(ENCRYPTION_FIRST_PATTERN_OFFSET)),
            Some(0)
        );
        assert_eq!(
            second.and_then(|offset| offset.checked_add_signed(ENCRYPTION_SECOND_PATTERN_OFFSET)),
            Some(4)
        );
    }

    fn encryption_fixture(value: u8) -> [u8; 36] {
        [
            value, 0x00, 0x00, 0x00, value, 0x00, 0x00, 0x00, 0xC8, 0xFD, 0x8E, 0x01, 0xC8, 0xFD,
            0x8E, 0x01, 0xD8, 0xFD, 0x8E, 0x01, 0xD8, 0xFD, 0x8E, 0x01, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x40, 0xFF, 0xFF, 0xFF, 0x3F,
        ]
    }

    fn find_pattern(bytes: &[u8], pattern: &str) -> Option<usize> {
        let tokens = pattern
            .split_whitespace()
            .map(|token| match token {
                "?" | "??" => Some(None),
                value => u8::from_str_radix(value, 16).ok().map(Some),
            })
            .collect::<Option<Vec<_>>>()?;

        bytes.windows(tokens.len()).position(|window| {
            window
                .iter()
                .zip(&tokens)
                .all(|(actual, expected)| expected.is_none_or(|value| *actual == value))
        })
    }
}
