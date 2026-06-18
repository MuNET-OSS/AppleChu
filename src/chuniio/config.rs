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
        let mut vk_ir = DEFAULT_IR;
        let mut vk_cell = DEFAULT_CELLS;

        for (idx, key) in vk_ir.iter_mut().enumerate() {
            let air_key = format!("air{}", idx + 1);
            let ir_key = format!("ir{}", idx + 1);
            *key =
                config.get_int_alias(&[("Air", &air_key), ("ir", &ir_key)], i64::from(*key)) as i32;
        }

        for (idx, key) in vk_cell.iter_mut().enumerate() {
            let cell_key = format!("cell{}", idx + 1);
            *key = config.get_int_alias(
                &[("Slider", &cell_key), ("slider", &cell_key)],
                i64::from(*key),
            ) as i32;
        }

        Self {
            vk_test: config.get_int_alias(&[("Buttons", "test"), ("io3", "test")], 0x70) as i32,
            vk_service: config.get_int_alias(&[("Buttons", "service"), ("io3", "service")], 0x71)
                as i32,
            vk_coin: config.get_int_alias(&[("Buttons", "coin"), ("io3", "coin")], 0x72) as i32,
            vk_ir_emu: config.get_int_alias(&[("Buttons", "ir"), ("io3", "ir")], 0x20) as i32,
            vk_ir,
            vk_cell,
        }
    }
}
