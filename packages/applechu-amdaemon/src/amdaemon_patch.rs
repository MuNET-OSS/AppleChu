use applechu::amdaemon::{AllowLocalhostConfig, CreditFreezeConfig};
use applechu::config::Config;
use applechu::patch_engine::{apply_patch, PatchVariant, VersionedPatch};
use applechu::util::api::Api;

const LOCALHOST_TEXT_ON: &str = "30 2F 38 00 31 32 37 2F 38 00 00 00 32 34 30 2F";
const LOCALHOST_TEXT_OFF: &str = "31 32 37 2F 38 00 00 00 32 34 30 2F";
const LOCALHOST_STUB: &str = "FF 15 ?? ?? ?? ?? 8B C0 48 83 C4 28 C3";
const LOCALHOST_STUB_ON: &[u8] = &[0x33, 0xC0, 0x48, 0x83, 0xC4, 0x28, 0xC3];
const CREDIT_FIELD: &str =
    "28 7C 4A 28 39 05 ?? ?? ?? ?? 74 05 0F B6 44 4A 29 88 84 11 C0 03 00 00";
const CREDIT_FIELD_ON: &str =
    "08 7C 4A 28 39 05 ?? ?? ?? ?? 74 05 0F B6 44 4A 29 88 84 11 C0 03 00 00";

pub(crate) fn apply_pre_tls() {
    let base_dir = crate::startup::executable_base_dir();
    let config = Config::global(&base_dir);
    let Some(api) = Api::standalone(crate::console::standalone_logger) else {
        return;
    };
    if config
        .section::<AllowLocalhostConfig>()
        .is_some_and(|section| section.enabled)
    {
        apply_localhost(&api);
    }
    if config
        .section::<CreditFreezeConfig>()
        .is_some_and(|section| section.enabled)
    {
        apply_credit_freeze(&api);
    }
}

fn apply_localhost(api: &Api) {
    apply_patch(
        api,
        &VersionedPatch {
            name: "AM Daemon localhost network server",
            variants: &[
                PatchVariant {
                    pattern: Some(LOCALHOST_TEXT_ON),
                    pattern_offset: 4,
                    known_offsets: &[0x732714, 0x69042C, 0x6E28A4],
                    expected: &[0x30, 0x2F, 0x38, 0x00],
                    patch: &[0x30, 0x2F, 0x38, 0x00],
                },
                PatchVariant {
                    pattern: Some(LOCALHOST_TEXT_OFF),
                    pattern_offset: 0,
                    known_offsets: &[0x732714, 0x69042C, 0x6E28A4],
                    expected: &[0x31, 0x32, 0x37, 0x2F],
                    patch: &[0x30, 0x2F, 0x38, 0x00],
                },
            ],
        },
    );
    apply_localhost_validation(api);
}

fn apply_localhost_validation(api: &Api) {
    let offsets = [0x3F6124, 0x53853, 0x539F6, 0x3C94C4];
    for offset in offsets {
        let addr = applechu::util::memory::file_offset_to_va(api, offset);
        if patch_localhost_validation(api, addr) {
            api.log_info("Patch applied: AM Daemon localhost validation");
            return;
        }
    }
    let Some((bytes, mask)) = parse_pattern(LOCALHOST_STUB) else {
        return;
    };
    let found = api.aob_scan(api.game_base(), api.game_size(), &bytes, &mask);
    if patch_localhost_validation(api, found) {
        api.log_info("Patch applied: AM Daemon localhost validation");
    } else {
        api.log_warn("Patch signature mismatch: AM Daemon localhost validation");
    }
}

fn patch_localhost_validation(api: &Api, addr: usize) -> bool {
    if addr == 0 {
        return false;
    }
    let mut current = [0; 13];
    if !api.mem_read(addr, &mut current) {
        return false;
    }
    if current[..LOCALHOST_STUB_ON.len()] == *LOCALHOST_STUB_ON {
        return true;
    }
    if current[0] != 0xFF
        || current[1] != 0x15
        || current[6..] != [0x8B, 0xC0, 0x48, 0x83, 0xC4, 0x28, 0xC3]
    {
        return false;
    }
    api.mem_write(addr, LOCALHOST_STUB_ON)
}

fn parse_pattern(pattern: &str) -> Option<(Vec<u8>, String)> {
    let mut bytes = Vec::new();
    let mut mask = String::new();
    for token in pattern.split_whitespace() {
        if token == "??" {
            bytes.push(0);
            mask.push('?');
        } else {
            bytes.push(u8::from_str_radix(token, 16).ok()?);
            mask.push('x');
        }
    }
    (!bytes.is_empty()).then_some((bytes, mask))
}

#[cfg(test)]
mod tests {
    use super::{parse_pattern, CREDIT_FIELD};

    #[test]
    fn credit_signature_targets_field_byte() {
        let (bytes, mask) = parse_pattern(CREDIT_FIELD).expect("签名必须有效");
        assert_eq!(bytes[0], 0x28);
        assert_eq!(&mask[16..17], "x");
        assert_eq!(bytes.len(), mask.len());
    }

    #[test]
    fn wildcard_signature_keeps_call_displacement_unchecked() {
        let (bytes, mask) =
            parse_pattern("FF 15 ?? ?? ?? ?? 8B C0 48 83 C4 28 C3").expect("签名必须有效");
        assert_eq!(bytes[0..2], [0xFF, 0x15]);
        assert_eq!(&mask[2..6], "????");
        assert_eq!(bytes[6..], [0x8B, 0xC0, 0x48, 0x83, 0xC4, 0x28, 0xC3]);
    }
}

fn apply_credit_freeze(api: &Api) {
    apply_patch(
        api,
        &VersionedPatch {
            name: "AM Daemon credit freeze",
            variants: &[
                PatchVariant {
                    pattern: Some(CREDIT_FIELD),
                    pattern_offset: 0,
                    known_offsets: &[0x2DD8F8, 0x2AB4E8, 0x2BBBC8],
                    expected: &[0x28],
                    patch: &[0x08],
                },
                PatchVariant {
                    pattern: Some(CREDIT_FIELD_ON),
                    pattern_offset: 0,
                    known_offsets: &[0x2DD8F8, 0x2AB4E8, 0x2BBBC8],
                    expected: &[0x08],
                    patch: &[0x08],
                },
            ],
        },
    );
}
