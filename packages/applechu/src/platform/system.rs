use std::fs;

use crate::config::Config;
use crate::patches::free_play::FreePlayConfig;
use crate::system_config::SystemConfig;
use crate::util::api::Api;

const SYSFILE_NAME: &str = "sysfile.dat";
const SYSFILE_SIZE: usize = 0x6000;
const BLOCK_SIZE: usize = 512;
const CREDIT_PRIMARY: usize = 0x0000;
const CREDIT_MIRROR: usize = 0x3000;
const DIP_PRIMARY: usize = 0x2800;
const DIP_MIRROR: usize = 0x5800;
const CREDIT_FREEPLAY_OFFSET: usize = 10;
const DIP_SWITCH_OFFSET: usize = 8;

#[applechu_macros::config_section(stage = Platform, order = 100)]
pub(crate) fn init(api: &Api, config: &Config, system: &SystemConfig) {
    let Some(mut path) = crate::platform::vfs::resolve_path(&format!("E:\\{SYSFILE_NAME}")) else {
        return;
    };
    if path.is_dir() {
        path.push(SYSFILE_NAME);
    }

    let Ok(mut data) = fs::read(&path) else {
        api.log_info("sysfile.dat not found; system settings will be written after first run");
        return;
    };
    if data.len() != SYSFILE_SIZE {
        return api
            .log_warn("sysfile.dat has an unexpected size; system settings were not written");
    }

    let freeplay = config
        .section::<FreePlayConfig>()
        .is_some_and(|config| config.enabled);
    let dipsw = system.dipsw();
    let dip_switches = dipsw_bits(dipsw);
    api.log_info(&format!(
        "System: Delivery Server: {}",
        if dipsw[0] { "Server" } else { "Client" },
    ));
    api.log_info(&format!(
        "System: Monitor Type: {}",
        if dipsw[1] { "60FPS" } else { "120FPS" },
    ));
    api.log_info(&format!(
        "System: Cabinet Type: {}",
        if dipsw[2] { "CVT" } else { "SP" },
    ));
    api.log_info(&format!(
        "System: DIPSW={}/{}/{} (0x{dip_switches:02X})",
        u8::from(dipsw[0]),
        u8::from(dipsw[1]),
        u8::from(dipsw[2]),
    ));
    let mut credit = data[CREDIT_PRIMARY..CREDIT_PRIMARY + BLOCK_SIZE].to_vec();
    let mut dip = data[DIP_PRIMARY..DIP_PRIMARY + BLOCK_SIZE].to_vec();
    credit[CREDIT_FREEPLAY_OFFSET] = u8::from(freeplay);
    dip[DIP_SWITCH_OFFSET] = dip_switches;
    update_checksum(&mut credit);
    update_checksum(&mut dip);

    for offset in [CREDIT_PRIMARY, CREDIT_MIRROR] {
        data[offset..offset + BLOCK_SIZE].copy_from_slice(&credit);
    }
    for offset in [DIP_PRIMARY, DIP_MIRROR] {
        data[offset..offset + BLOCK_SIZE].copy_from_slice(&dip);
    }

    if fs::write(&path, data).is_ok() {
        api.log_info("Updated free-play and DIP switch settings in sysfile.dat");
    } else {
        api.log_warn("Failed to write sysfile.dat");
    }
}

fn dipsw_bits(switches: [bool; 3]) -> u8 {
    switches
        .into_iter()
        .enumerate()
        .fold(0, |bits, (index, enabled)| {
            bits | (u8::from(enabled) << index)
        })
}

fn update_checksum(block: &mut [u8]) {
    let checksum = crc32(&block[4..]);
    block[..4].copy_from_slice(&checksum.to_le_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_the_same_crc32_used_by_segtools() {
        let mut block = vec![0; BLOCK_SIZE];
        block[CREDIT_FREEPLAY_OFFSET] = 1;
        update_checksum(&mut block);
        assert_eq!(
            u32::from_le_bytes(block[..4].try_into().unwrap()),
            crc32(&block[4..])
        );
    }
}
