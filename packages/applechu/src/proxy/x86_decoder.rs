pub(super) unsafe fn instruction_span(start: *const u8, min_len: usize) -> Option<usize> {
    let mut total = 0;
    while total < min_len {
        let len = instruction_len(start.add(total))?;
        if len == 0 || total + len > 16 {
            return None;
        }
        total += len;
    }
    Some(total)
}

unsafe fn instruction_len(mut code: *const u8) -> Option<usize> {
    let start = code;
    while let 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3 = *code {
        code = code.add(1);
    }

    let opcode = *code;
    code = code.add(1);
    let mut has_modrm = false;
    let mut imm_len = 0usize;

    match opcode {
        0x00..=0x03
        | 0x08..=0x0B
        | 0x10..=0x13
        | 0x18..=0x1B
        | 0x20..=0x23
        | 0x28..=0x2B
        | 0x30..=0x33
        | 0x38..=0x3B
        | 0x62
        | 0x63
        | 0x69
        | 0x6B
        | 0x84..=0x8F
        | 0xC0
        | 0xC1
        | 0xC4
        | 0xC5
        | 0xD0..=0xD3
        | 0xF6
        | 0xF7
        | 0xFE
        | 0xFF => {
            has_modrm = true;
            imm_len = match opcode {
                0x69 => 4,
                0x6B | 0xC0 | 0xC1 => 1,
                _ => 0,
            };
        }
        0x04
        | 0x0C
        | 0x14
        | 0x1C
        | 0x24
        | 0x2C
        | 0x34
        | 0x3C
        | 0x6A
        | 0xA8
        | 0xB0..=0xB7
        | 0xC2
        | 0xCA
        | 0xCD
        | 0xD4
        | 0xD5
        | 0xE0..=0xE3
        | 0xEB => imm_len = 1,
        0x05
        | 0x0D
        | 0x15
        | 0x1D
        | 0x25
        | 0x2D
        | 0x35
        | 0x3D
        | 0x68
        | 0xA0..=0xA3
        | 0xA9
        | 0xB8..=0xBF
        | 0xC7
        | 0xE8
        | 0xE9 => {
            has_modrm = opcode == 0xC7;
            imm_len = 4;
        }
        0x70..=0x7F => imm_len = 1,
        0x90..=0x9F
        | 0x50..=0x5F
        | 0x6C..=0x6F
        | 0xA4..=0xA7
        | 0xAA..=0xAF
        | 0xC3
        | 0xC9
        | 0xCB
        | 0xCC
        | 0xCE
        | 0xCF
        | 0xF4
        | 0xF5
        | 0xF8..=0xFD => {}
        0x0F => {
            let second = *code;
            code = code.add(1);
            match second {
                0x80..=0x8F => imm_len = 4,
                0x90..=0x9F
                | 0xA3..=0xA5
                | 0xAB
                | 0xAD
                | 0xAF
                | 0xB0..=0xB7
                | 0xBA
                | 0xBB
                | 0xBC
                | 0xBD
                | 0xBE
                | 0xBF
                | 0xC0
                | 0xC1 => {
                    has_modrm = true;
                    imm_len = if matches!(second, 0xA4 | 0xAC | 0xBA) {
                        1
                    } else {
                        0
                    };
                }
                _ => return None,
            }
        }
        _ => return Some(code.offset_from(start) as usize),
    }

    if has_modrm {
        code = code.add(modrm_len(code)?);
    }
    Some(code.offset_from(start) as usize + imm_len)
}

unsafe fn modrm_len(code: *const u8) -> Option<usize> {
    let modrm = *code;
    let mode = modrm >> 6;
    let rm = modrm & 7;
    let mut len = 1usize;

    if mode != 3 && rm == 4 {
        let sib = *code.add(len);
        len += 1;
        if mode == 0 && (sib & 7) == 5 {
            len += 4;
        }
    }

    match mode {
        0 if rm == 5 => len += 4,
        1 => len += 1,
        2 => len += 4,
        0 | 3 => {}
        _ => return None,
    }

    Some(len)
}
