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
            vk_test: io4.test.code(),
            vk_service: io4.service.code(),
            vk_coin: io4.coin.code(),
            vk_ir_emu: io4.ir.code(),
            vk_ir: [
                io4.air1.code(),
                io4.air2.code(),
                io4.air3.code(),
                io4.air4.code(),
                io4.air5.code(),
                io4.air6.code(),
            ],
            vk_cell: [
                io4.cell1.code(),
                io4.cell2.code(),
                io4.cell3.code(),
                io4.cell4.code(),
                io4.cell5.code(),
                io4.cell6.code(),
                io4.cell7.code(),
                io4.cell8.code(),
                io4.cell9.code(),
                io4.cell10.code(),
                io4.cell11.code(),
                io4.cell12.code(),
                io4.cell13.code(),
                io4.cell14.code(),
                io4.cell15.code(),
                io4.cell16.code(),
                io4.cell17.code(),
                io4.cell18.code(),
                io4.cell19.code(),
                io4.cell20.code(),
                io4.cell21.code(),
                io4.cell22.code(),
                io4.cell23.code(),
                io4.cell24.code(),
                io4.cell25.code(),
                io4.cell26.code(),
                io4.cell27.code(),
                io4.cell28.code(),
                io4.cell29.code(),
                io4.cell30.code(),
                io4.cell31.code(),
                io4.cell32.code(),
            ],
        }
    }
}
