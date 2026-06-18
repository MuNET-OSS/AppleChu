use std::fs;

use crate::config::Config;
use crate::util::api::Api;

const SYSFILE_NAME: &str = "sysfile.dat";

pub fn init(api: &Api, config: &Config) {
    if config.get_bool("System", "enable", true) == false {
        return;
    }

    let Some(mut path) = crate::platform::vfs::resolve_path(&format!("E:\\{}", SYSFILE_NAME))
    else {
        return;
    };
    if path.is_dir() {
        path.push(SYSFILE_NAME);
    }

    let Ok(mut data) = fs::read(&path) else {
        api.log_warn("system: sysfile.dat not found, dipsw patch skipped");
        return;
    };

    let mut changed = false;
    let freeplay = config.get_bool("System", "freeplay", config.is_enabled("FreePlay"));
    if patch_token(&mut data, b"freeplay", u8::from(freeplay)) {
        changed = true;
    }

    let dipsw = dipsw_bits(config);
    if patch_token(&mut data, b"dip_switches", dipsw) || patch_token(&mut data, b"dipsw", dipsw) {
        changed = true;
    }

    if changed {
        if fs::write(&path, data).is_ok() {
            api.log_info("system: sysfile.dat freeplay/dipsw updated");
        } else {
            api.log_warn("system: failed to write sysfile.dat");
        }
    }
}

fn dipsw_bits(config: &Config) -> u8 {
    let mut bits = 0u8;
    for index in 1..=3 {
        if config.get_bool("System", &format!("dipsw{}", index), false) {
            bits |= 1 << (index - 1);
        }
    }
    bits
}

fn patch_token(data: &mut [u8], token: &[u8], value: u8) -> bool {
    let Some(pos) = find_bytes(data, token) else {
        return false;
    };
    let Some(slot) = data.get_mut(pos + token.len()) else {
        return false;
    };
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
