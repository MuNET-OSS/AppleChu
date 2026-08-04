use super::{Io4Ops, Io4State};
use super::{BUTTON_SERVICE, BUTTON_TEST};
use crate::chuniio;

// API < 0x0101 的旧版映射使用反向 IR beam 顺序
// 报告该版本的 DLL 按旧映射写入，因此必须保留兼容
const IR_MASKS_V1: [(u16, u16); 6] = [
    (0, 1 << 13),
    (1 << 13, 0),
    (0, 1 << 12),
    (1 << 12, 0),
    (0, 1 << 11),
    (1 << 11, 0),
];

const IR_MASKS: [(u16, u16); 6] = [
    (1 << 13, 0),
    (0, 1 << 13),
    (1 << 12, 0),
    (0, 1 << 12),
    (1 << 11, 0),
    (0, 1 << 11),
];

pub struct ChusanIo4Ops;

impl Io4Ops for ChusanIo4Ops {
    fn poll(&self) -> Io4State {
        let mut state = Io4State::default();
        let mut opbtn = 0;
        let mut beams = 0;

        chuniio::jvs_poll(&mut opbtn, &mut beams);
        let coins = chuniio::jvs_read_coin_counter();

        if opbtn & chuniio::OPBTN_TEST != 0 {
            state.buttons[0] |= BUTTON_TEST;
        }
        if opbtn & chuniio::OPBTN_SERVICE != 0 {
            state.buttons[0] |= BUTTON_SERVICE;
        }
        state.chutes[0] = coins << 8;

        let masks = if chuniio::effective_api_version() >= 0x0101 {
            &IR_MASKS
        } else {
            &IR_MASKS_V1
        };
        for (idx, (p1, p2)) in masks.iter().copied().enumerate() {
            if beams & (1 << idx) == 0 {
                state.buttons[0] |= p1;
                state.buttons[1] |= p2;
            }
        }

        state
    }
}
