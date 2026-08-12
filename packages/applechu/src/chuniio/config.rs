use crate::config::Config;

pub const DEFAULT_CELLS: [i32; 32] = [
    b'L' as i32,
    b'L' as i32,
    b'L' as i32,
    b'L' as i32,
    b'K' as i32,
    b'K' as i32,
    b'K' as i32,
    b'K' as i32,
    b'J' as i32,
    b'J' as i32,
    b'J' as i32,
    b'J' as i32,
    b'H' as i32,
    b'H' as i32,
    b'H' as i32,
    b'H' as i32,
    b'G' as i32,
    b'G' as i32,
    b'G' as i32,
    b'G' as i32,
    b'F' as i32,
    b'F' as i32,
    b'F' as i32,
    b'F' as i32,
    b'D' as i32,
    b'D' as i32,
    b'D' as i32,
    b'D' as i32,
    b'S' as i32,
    b'S' as i32,
    b'S' as i32,
    b'S' as i32,
];

pub const DEFAULT_IR: [i32; 6] = [
    b'4' as i32,
    b'5' as i32,
    b'6' as i32,
    b'7' as i32,
    b'8' as i32,
    b'9' as i32,
];

#[derive(Clone)]
pub struct ChuniIoConfig {
    pub vk_test: i32,
    pub vk_service: i32,
    pub vk_coin: i32,
    pub vk_ir_emu: i32,
    pub vk_ir: [i32; 6],
    pub vk_cell: [i32; 32],
}

impl ChuniIoConfig {
    pub fn load(config: &Config) -> Self {
        let io4 = config
            .section::<crate::io4::Io4Config>()
            .map_or_else(crate::io4::Io4Config::default, |value| (*value).clone());

        Self {
            vk_test: io4.test,
            vk_service: io4.service,
            vk_coin: io4.coin,
            vk_ir_emu: io4.ir,
            vk_ir: [io4.air1, io4.air2, io4.air3, io4.air4, io4.air5, io4.air6],
            vk_cell: [
                io4.cell1, io4.cell2, io4.cell3, io4.cell4, io4.cell5, io4.cell6, io4.cell7,
                io4.cell8, io4.cell9, io4.cell10, io4.cell11, io4.cell12, io4.cell13, io4.cell14,
                io4.cell15, io4.cell16, io4.cell17, io4.cell18, io4.cell19, io4.cell20, io4.cell21,
                io4.cell22, io4.cell23, io4.cell24, io4.cell25, io4.cell26, io4.cell27, io4.cell28,
                io4.cell29, io4.cell30, io4.cell31, io4.cell32,
            ],
        }
    }
}
