use std::fs;
use std::path::PathBuf;

use super::external::ExternalAimeIo;
use super::{external_call, AimeReader};

const SYNC: u8 = 0xE0;
const ESC: u8 = 0xD0;

const CMD_GET_FW_VERSION: u8 = 0x30;
const CMD_GET_HW_VERSION: u8 = 0x32;
const CMD_RADIO_ON: u8 = 0x40;
const CMD_RADIO_OFF: u8 = 0x41;
const CMD_POLL: u8 = 0x42;
const CMD_MIFARE_SELECT_TAG: u8 = 0x43;
const CMD_MIFARE_SET_KEY_AIME: u8 = 0x50;
const CMD_MIFARE_AUTHENTICATE_AIME: u8 = 0x51;
const CMD_MIFARE_READ_BLOCK: u8 = 0x52;
const CMD_MIFARE_SET_KEY_BANA: u8 = 0x54;
const CMD_MIFARE_AUTHENTICATE_BANA: u8 = 0x55;
const CMD_TO_UPDATE_MODE: u8 = 0x60;
const CMD_SEND_HEX_DATA: u8 = 0x61;
const CMD_RESET: u8 = 0x62;
const CMD_FELICA_ENCAP: u8 = 0x71;

const LED_ADDR: u8 = 0x08;
const LED_CMD_SET_COLOR: u8 = 0x81;
const LED_CMD_GET_INFO: u8 = 0xF0;
const LED_CMD_RESET: u8 = 0xF5;

const KEY_AIME: u8 = 0;
const KEY_BANA: u8 = 1;
const FELICA_PMM: u64 = 0x00F1000000014300;
const FELICA_SYSTEM_CODE: u16 = 0x88B4;

const HW_VERSION: [&[u8]; 3] = [b"TN32MSEC003S H/W Ver3.0", b"837-15286", b"837-15396"];
const FW_VERSION: [&[u8]; 3] = [b"TN32MSEC003S F/W Ver1.2", b"\x94", b"\x94"];
const LED_VERSION: [&[u8]; 3] = [
    b"15084\xFF\x10\x00\x12",
    b"000-00000\xFF\x11\x40",
    b"000-00000\xFF\x11\x40",
];

#[derive(Clone)]
pub struct SgNfcConfig {
    pub gen: u8,
    pub proxy_flag: u8,
    pub authdata_path: PathBuf,
}

pub struct SgNfcDevice {
    addr: u8,
    gen: u8,
    proxy_flag: u8,
    authdata_path: PathBuf,
    mifare: [[[u8; 16]; 4]; 16],
    felica_idm: u64,
    rx: Vec<u8>,
    tx: Vec<u8>,
}

impl SgNfcDevice {
    pub fn new(config: SgNfcConfig) -> Self {
        Self {
            addr: 0,
            gen: config.gen.clamp(1, 3),
            proxy_flag: config.proxy_flag,
            authdata_path: config.authdata_path,
            mifare: [[[0; 16]; 4]; 16],
            felica_idm: 0,
            rx: Vec::new(),
            tx: Vec::new(),
        }
    }

    pub fn write(&mut self, reader: &AimeReader, bytes: &[u8]) {
        self.rx.extend_from_slice(bytes);
        while let Some(frame) = decode_frame(&mut self.rx) {
            if let Some(response) = self.dispatch(reader, &frame) {
                encode_frame_into(&mut self.tx, &response);
            }
        }
    }

    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let count = out.len().min(self.tx.len());
        out[..count].copy_from_slice(&self.tx[..count]);
        self.tx.drain(..count);
        count
    }

    fn dispatch(&mut self, reader: &AimeReader, req: &[u8]) -> Option<Vec<u8>> {
        if req.len() < 5 || req[0] as usize != req.len() {
            return None;
        }
        let addr = req[1];
        let payload_len = req[4] as usize;
        if payload_len != req.len().saturating_sub(5) {
            return None;
        }

        let seq = req[2];
        let cmd = req[3];
        let payload = &req[5..];
        if addr == LED_ADDR {
            return self.dispatch_led(seq, cmd, payload);
        }
        if addr != self.addr {
            return None;
        }
        let result = match cmd {
            CMD_RESET => Ok(response(addr, seq, cmd, 3, &[])),
            CMD_GET_FW_VERSION => Ok(self.version_response(addr, seq, cmd, true)),
            CMD_GET_HW_VERSION => Ok(self.version_response(addr, seq, cmd, false)),
            CMD_POLL => self.cmd_poll(reader, addr, seq, cmd),
            CMD_MIFARE_READ_BLOCK => self.cmd_mifare_read_block(reader, addr, seq, cmd, payload),
            CMD_FELICA_ENCAP => self.cmd_felica_encap(reader, addr, seq, cmd, payload),
            CMD_MIFARE_SELECT_TAG => self.cmd_mifare_select(addr, seq, cmd, payload),
            CMD_MIFARE_SET_KEY_AIME => self.cmd_mifare_set_key(addr, seq, cmd, KEY_AIME, payload),
            CMD_MIFARE_SET_KEY_BANA => self.cmd_mifare_set_key(addr, seq, cmd, KEY_BANA, payload),
            CMD_MIFARE_AUTHENTICATE_AIME => {
                self.cmd_mifare_authenticate(addr, seq, cmd, KEY_AIME, payload)
            }
            CMD_MIFARE_AUTHENTICATE_BANA => {
                self.cmd_mifare_authenticate(addr, seq, cmd, KEY_BANA, payload)
            }
            CMD_RADIO_ON => {
                self.cmd_unit(addr, seq, cmd, |external| unsafe { external.radio_on() })
            }
            CMD_RADIO_OFF => {
                self.cmd_unit(addr, seq, cmd, |external| unsafe { external.radio_off() })
            }
            CMD_TO_UPDATE_MODE => self.cmd_unit(addr, seq, cmd, |external| unsafe {
                external.to_update_mode()
            }),
            CMD_SEND_HEX_DATA => self.cmd_send_hex_data(addr, seq, cmd, payload),
            _ => Err(()),
        };

        Some(result.unwrap_or_else(|_| response(addr, seq, cmd, 1, &[])))
    }

    fn dispatch_led(&self, seq: u8, cmd: u8, payload: &[u8]) -> Option<Vec<u8>> {
        match cmd {
            LED_CMD_RESET => Some(response(LED_ADDR, seq, cmd, 0, &[0])),
            LED_CMD_GET_INFO => {
                let index = self.gen.saturating_sub(1) as usize;
                Some(response(LED_ADDR, seq, cmd, 0, LED_VERSION[index]))
            }
            LED_CMD_SET_COLOR => {
                if payload.len() == 3 {
                    external_call(|external| unsafe {
                        external.led_set_color(payload[0], payload[1], payload[2]);
                        0
                    });
                }
                None
            }
            _ => Some(response(LED_ADDR, seq, cmd, 1, &[])),
        }
    }

    fn version_response(&self, addr: u8, seq: u8, cmd: u8, firmware: bool) -> Vec<u8> {
        let index = self.gen.saturating_sub(1) as usize;
        let data = if firmware {
            FW_VERSION[index]
        } else {
            HW_VERSION[index]
        };
        response(addr, seq, cmd, 0, data)
    }

    fn cmd_poll(&mut self, reader: &AimeReader, addr: u8, seq: u8, cmd: u8) -> Result<Vec<u8>, ()> {
        external_call(|external| unsafe { external.poll() });

        if let Some(idm) = reader.read_felica_id() {
            self.felica_idm = idm;
            let mut payload = Vec::with_capacity(19);
            payload.push(1);
            payload.push(0x20);
            payload.push(16);
            payload.extend_from_slice(&idm.swap_bytes().to_ne_bytes());
            payload.extend_from_slice(&FELICA_PMM.swap_bytes().to_ne_bytes());
            return Ok(response(addr, seq, cmd, 0, &payload));
        }

        if let Some(luid) = reader.read_aime_id() {
            self.populate_aime_card(&luid)?;
            let uid = reader.read_mifare_uid().unwrap_or([1, 2, 3, 4]);
            let mut payload = Vec::with_capacity(7);
            payload.push(1);
            payload.push(0x10);
            payload.push(4);
            payload.extend_from_slice(&uid);
            return Ok(response(addr, seq, cmd, 0, &payload));
        }

        Ok(response(addr, seq, cmd, 0, &[0]))
    }

    fn cmd_mifare_read_block(
        &self,
        reader: &AimeReader,
        addr: u8,
        seq: u8,
        cmd: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, ()> {
        if payload.len() != 5 {
            return Err(());
        }
        let uid = [payload[0], payload[1], payload[2], payload[3]];
        let block_no = payload[4];
        let mut block = [0u8; 16];

        if reader.mifare_read_block(&uid, block_no, &mut block) {
            return Ok(response(addr, seq, cmd, 0, &block));
        }

        if block_no > 14 {
            return Err(());
        }

        if block_no == 4 {
            block[0] = 0x54;
            block[1] = 0x43;
            block[2] = self.proxy_flag;
            block[3] = 0x01;
        } else if block_no >= 5 {
            self.read_authdata_block(block_no, &mut block);
        } else {
            block = self.mifare[0][block_no as usize];
        }

        Ok(response(addr, seq, cmd, 0, &block))
    }

    fn cmd_felica_encap(
        &mut self,
        reader: &AimeReader,
        addr: u8,
        seq: u8,
        cmd: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, ()> {
        if payload.len() < 9 || payload[8] as usize + 8 != payload.len() {
            return Err(());
        }
        let felica_req = &payload[8..];
        let mut felica_res = [0u8; 250];
        if let Some(count) = reader.felica_transact(felica_req, &mut felica_res) {
            return Ok(response(addr, seq, cmd, 0, &felica_res[..count]));
        }

        let count = self.felica_transact(felica_req, &mut felica_res)?;
        Ok(response(addr, seq, cmd, 0, &felica_res[..count]))
    }

    fn cmd_mifare_select(&self, addr: u8, seq: u8, cmd: u8, payload: &[u8]) -> Result<Vec<u8>, ()> {
        if payload.len() != 4 {
            return Err(());
        }
        external_call(|external| unsafe { external.mifare_select(payload) });
        Ok(response(addr, seq, cmd, 0, &[]))
    }

    fn cmd_mifare_set_key(
        &self,
        addr: u8,
        seq: u8,
        cmd: u8,
        key_type: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, ()> {
        external_call(|external| unsafe { external.mifare_set_key(key_type, payload) });
        Ok(response(addr, seq, cmd, 0, &[]))
    }

    fn cmd_mifare_authenticate(
        &self,
        addr: u8,
        seq: u8,
        cmd: u8,
        key_type: u8,
        payload: &[u8],
    ) -> Result<Vec<u8>, ()> {
        external_call(|external| unsafe { external.mifare_authenticate(key_type, payload) });
        Ok(response(addr, seq, cmd, 0, &[]))
    }

    fn cmd_unit<F>(&self, addr: u8, seq: u8, cmd: u8, call: F) -> Result<Vec<u8>, ()>
    where
        F: FnOnce(&ExternalAimeIo) -> i32,
    {
        external_call(call);
        Ok(response(addr, seq, cmd, 0, &[]))
    }

    fn cmd_send_hex_data(&self, addr: u8, seq: u8, cmd: u8, payload: &[u8]) -> Result<Vec<u8>, ()> {
        let mut status = if payload.len() == 0x2B { 0x20 } else { 0 };
        external_call(|external| unsafe { external.send_hex_data(payload, &mut status) });
        Ok(response(addr, seq, cmd, status, &[]))
    }

    fn populate_aime_card(&mut self, luid: &[u8; 10]) -> Result<(), ()> {
        self.mifare = [[[0; 16]; 4]; 16];
        for byte in luid {
            if (byte & 0xF0) > 0x90 || (byte & 0x0F) > 0x09 {
                return Err(());
            }
        }
        self.mifare[0][2][6..16].copy_from_slice(luid);
        Ok(())
    }

    fn read_authdata_block(&self, block_no: u8, block: &mut [u8; 16]) {
        let offset = match block_no {
            6 => 16,
            8 => 32,
            9 => 48,
            10 => 64,
            12 => 82,
            13 => 98,
            14 => 114,
            _ => 0,
        };
        if let Ok(authdata) = fs::read(&self.authdata_path) {
            for (idx, byte) in block.iter_mut().enumerate() {
                if let Some(value) = authdata.get(offset + idx) {
                    *byte = *value;
                }
            }
        }
    }

    fn felica_transact(&self, req: &[u8], res: &mut [u8; 250]) -> Result<usize, ()> {
        if req.is_empty() || req[0] as usize != req.len() || req.len() < 2 {
            return Err(());
        }
        let mut pos = 1;
        let code = read_u8(req, &mut pos)?;
        let mut out = Vec::with_capacity(64);
        out.push(0);
        out.push(code.wrapping_add(1));

        if code != 0x00 {
            let idm = read_be_u64(req, &mut pos)?;
            if idm != self.felica_idm {
                return Err(());
            }
            out.extend_from_slice(&idm.to_be_bytes());
        }

        match code {
            0x00 => self.felica_poll(req, &mut pos, &mut out)?,
            0x0C => {
                out.push(1);
                out.extend_from_slice(&FELICA_SYSTEM_CODE.to_be_bytes());
            }
            0x06 => self.felica_read_without_encryption(req, &mut pos, &mut out)?,
            0x08 => out.extend_from_slice(&0u16.to_be_bytes()),
            0xA4 => out.push(0),
            _ => return Err(()),
        }

        if out.len() > res.len() {
            return Err(());
        }
        out[0] = out.len() as u8;
        res[..out.len()].copy_from_slice(&out);
        Ok(out.len())
    }

    fn felica_poll(&self, req: &[u8], pos: &mut usize, out: &mut Vec<u8>) -> Result<(), ()> {
        let system_code = read_be_u16(req, pos)?;
        let request_code = read_u8(req, pos)?;
        let _time_slot = read_u8(req, pos)?;
        if system_code != 0xFFFF && system_code != FELICA_SYSTEM_CODE {
            return Err(());
        }
        out.extend_from_slice(&self.felica_idm.to_be_bytes());
        out.extend_from_slice(&FELICA_PMM.to_be_bytes());
        if request_code == 1 {
            out.extend_from_slice(&FELICA_SYSTEM_CODE.to_be_bytes());
        }
        Ok(())
    }

    fn felica_read_without_encryption(
        &self,
        req: &[u8],
        pos: &mut usize,
        out: &mut Vec<u8>,
    ) -> Result<(), ()> {
        let service_count = read_u8(req, pos)? as usize;
        for _ in 0..service_count {
            let _service = read_be_u16(req, pos)?;
        }
        let block_count = read_u8(req, pos)? as usize;
        let mut blocks = Vec::with_capacity(block_count);
        for _ in 0..block_count {
            let _access = read_u8(req, pos)?;
            blocks.push(read_u8(req, pos)?);
        }

        out.extend_from_slice(&0u16.to_be_bytes());
        out.push(block_count as u8);
        for block in blocks {
            if block == 0x82 {
                out.extend_from_slice(&self.felica_idm.to_be_bytes());
                out.extend_from_slice(&0x0078000000000000u64.to_be_bytes());
            } else {
                out.extend_from_slice(&[0; 16]);
            }
        }
        Ok(())
    }
}

fn response(addr: u8, seq: u8, cmd: u8, status: u8, payload: &[u8]) -> Vec<u8> {
    let frame_len = 6usize.saturating_add(payload.len()).min(255);
    let payload_len = frame_len.saturating_sub(6);
    let mut out = Vec::with_capacity(frame_len);
    out.push(frame_len as u8);
    out.push(addr);
    out.push(seq);
    out.push(cmd);
    out.push(status);
    out.push(payload_len as u8);
    out.extend_from_slice(&payload[..payload_len]);
    out
}

fn decode_frame(rx: &mut Vec<u8>) -> Option<Vec<u8>> {
    let sync = rx.iter().position(|byte| *byte == SYNC)?;
    if sync > 0 {
        rx.drain(..sync);
    }
    let mut decoded = Vec::with_capacity(256);
    let mut escape = false;
    for (idx, byte) in rx.iter().copied().enumerate().skip(1) {
        if byte == SYNC {
            rx.drain(..idx);
            return None;
        }
        if byte == ESC {
            escape = true;
            continue;
        }
        decoded.push(if escape { byte.wrapping_add(1) } else { byte });
        escape = false;
        if let Some(frame_len) = decoded.first().copied() {
            if decoded.len() == frame_len as usize + 1 {
                let checksum = decoded[..decoded.len() - 1]
                    .iter()
                    .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
                rx.drain(..=idx);
                if checksum == decoded[decoded.len() - 1] {
                    decoded.pop();
                    return Some(decoded);
                }
                return None;
            }
        }
    }
    None
}

fn encode_frame_into(tx: &mut Vec<u8>, frame: &[u8]) {
    tx.push(SYNC);
    let checksum = frame.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    for byte in frame.iter().copied().chain(std::iter::once(checksum)) {
        if byte == SYNC || byte == ESC {
            tx.push(ESC);
            tx.push(byte.wrapping_sub(1));
        } else {
            tx.push(byte);
        }
    }
}

fn read_u8(req: &[u8], pos: &mut usize) -> Result<u8, ()> {
    let value = *req.get(*pos).ok_or(())?;
    *pos += 1;
    Ok(value)
}

fn read_be_u16(req: &[u8], pos: &mut usize) -> Result<u16, ()> {
    let hi = read_u8(req, pos)?;
    let lo = read_u8(req, pos)?;
    Ok(u16::from_be_bytes([hi, lo]))
}

fn read_be_u64(req: &[u8], pos: &mut usize) -> Result<u64, ()> {
    let mut bytes = [0u8; 8];
    for byte in &mut bytes {
        *byte = read_u8(req, pos)?;
    }
    Ok(u64::from_be_bytes(bytes))
}
