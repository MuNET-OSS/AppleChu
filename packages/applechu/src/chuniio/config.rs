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

crate::config_section! {
    pub(crate) struct ButtonsConfig => BUTTONS_CONFIG_SECTION {
        section: "Buttons",
        order: 310,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "机台按钮按键映射",
        fields: {
            pub test: i32 = 0x70,
            comment: "测试按钮虚拟键码";
            pub service: i32 = 0x71,
            comment: "服务按钮虚拟键码";
            pub coin: i32 = 0x72,
            comment: "投币按钮虚拟键码";
            pub ir: i32 = 0x20,
            comment: "红外模拟虚拟键码";
        }
    }
}

crate::config_section! {
    pub(crate) struct AirConfig => AIR_CONFIG_SECTION {
        section: "Air",
        order: 311,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "红外传感器按键映射",
        fields: {
            pub air1: i32 = b'4' as i32,
            comment: "第 1 组红外传感器按键";
            pub air2: i32 = b'5' as i32,
            comment: "第 2 组红外传感器按键";
            pub air3: i32 = b'6' as i32,
            comment: "第 3 组红外传感器按键";
            pub air4: i32 = b'7' as i32,
            comment: "第 4 组红外传感器按键";
            pub air5: i32 = b'8' as i32,
            comment: "第 5 组红外传感器按键";
            pub air6: i32 = b'9' as i32,
            comment: "第 6 组红外传感器按键";
        }
    }
}

crate::config_section! {
    pub(crate) struct SliderKeysConfig => SLIDER_KEYS_CONFIG_SECTION {
        section: "Slider",
        order: 312,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "触摸条按键映射",
        fields: {
            pub cell1: i32 = b'L' as i32,
            comment: "触摸条第 1 单元按键";
            pub cell2: i32 = b'L' as i32,
            comment: "触摸条第 2 单元按键";
            pub cell3: i32 = b'L' as i32,
            comment: "触摸条第 3 单元按键";
            pub cell4: i32 = b'L' as i32,
            comment: "触摸条第 4 单元按键";
            pub cell5: i32 = b'K' as i32,
            comment: "触摸条第 5 单元按键";
            pub cell6: i32 = b'K' as i32,
            comment: "触摸条第 6 单元按键";
            pub cell7: i32 = b'K' as i32,
            comment: "触摸条第 7 单元按键";
            pub cell8: i32 = b'K' as i32,
            comment: "触摸条第 8 单元按键";
            pub cell9: i32 = b'J' as i32,
            comment: "触摸条第 9 单元按键";
            pub cell10: i32 = b'J' as i32,
            comment: "触摸条第 10 单元按键";
            pub cell11: i32 = b'J' as i32,
            comment: "触摸条第 11 单元按键";
            pub cell12: i32 = b'J' as i32,
            comment: "触摸条第 12 单元按键";
            pub cell13: i32 = b'H' as i32,
            comment: "触摸条第 13 单元按键";
            pub cell14: i32 = b'H' as i32,
            comment: "触摸条第 14 单元按键";
            pub cell15: i32 = b'H' as i32,
            comment: "触摸条第 15 单元按键";
            pub cell16: i32 = b'H' as i32,
            comment: "触摸条第 16 单元按键";
            pub cell17: i32 = b'G' as i32,
            comment: "触摸条第 17 单元按键";
            pub cell18: i32 = b'G' as i32,
            comment: "触摸条第 18 单元按键";
            pub cell19: i32 = b'G' as i32,
            comment: "触摸条第 19 单元按键";
            pub cell20: i32 = b'G' as i32,
            comment: "触摸条第 20 单元按键";
            pub cell21: i32 = b'F' as i32,
            comment: "触摸条第 21 单元按键";
            pub cell22: i32 = b'F' as i32,
            comment: "触摸条第 22 单元按键";
            pub cell23: i32 = b'F' as i32,
            comment: "触摸条第 23 单元按键";
            pub cell24: i32 = b'F' as i32,
            comment: "触摸条第 24 单元按键";
            pub cell25: i32 = b'D' as i32,
            comment: "触摸条第 25 单元按键";
            pub cell26: i32 = b'D' as i32,
            comment: "触摸条第 26 单元按键";
            pub cell27: i32 = b'D' as i32,
            comment: "触摸条第 27 单元按键";
            pub cell28: i32 = b'D' as i32,
            comment: "触摸条第 28 单元按键";
            pub cell29: i32 = b'S' as i32,
            comment: "触摸条第 29 单元按键";
            pub cell30: i32 = b'S' as i32,
            comment: "触摸条第 30 单元按键";
            pub cell31: i32 = b'S' as i32,
            comment: "触摸条第 31 单元按键";
            pub cell32: i32 = b'S' as i32,
            comment: "触摸条第 32 单元按键";
        }
    }
}

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
        let buttons = config
            .section::<ButtonsConfig>()
            .map_or_else(ButtonsConfig::default, |value| (*value).clone());
        let air = config
            .section::<AirConfig>()
            .map_or_else(AirConfig::default, |value| (*value).clone());
        let slider = config
            .section::<SliderKeysConfig>()
            .map_or_else(SliderKeysConfig::default, |value| (*value).clone());

        Self {
            vk_test: buttons.test,
            vk_service: buttons.service,
            vk_coin: buttons.coin,
            vk_ir_emu: buttons.ir,
            vk_ir: [air.air1, air.air2, air.air3, air.air4, air.air5, air.air6],
            vk_cell: [
                slider.cell1,
                slider.cell2,
                slider.cell3,
                slider.cell4,
                slider.cell5,
                slider.cell6,
                slider.cell7,
                slider.cell8,
                slider.cell9,
                slider.cell10,
                slider.cell11,
                slider.cell12,
                slider.cell13,
                slider.cell14,
                slider.cell15,
                slider.cell16,
                slider.cell17,
                slider.cell18,
                slider.cell19,
                slider.cell20,
                slider.cell21,
                slider.cell22,
                slider.cell23,
                slider.cell24,
                slider.cell25,
                slider.cell26,
                slider.cell27,
                slider.cell28,
                slider.cell29,
                slider.cell30,
                slider.cell31,
                slider.cell32,
            ],
        }
    }
}
