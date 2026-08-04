//! 全国对战曲库读取：hook 游戏曲库加载函数，遍历 std::map 红黑树导出曲库给 reflector 求交集。
//!
//! 机制对照 duolinguo（闭源，经 Ghidra 逆向）在 SDHD245 上验证：
//! - 签名 `50 E8 ?? ?? ?? ?? 8D 8D E8 FE FF FF E8` 定位曲库加载 CALL 点
//! - 解 E8 rel32 得加载函数地址，inline hook 之
//! - detour 先调原函数，再遍历 std::map<id, MusicInfo> 红黑树
//! - 每曲：id = node+0x10 (u16)，难度 vector = node+0x208..0x20c（每项 0x40 字节，[+0]=难度id [+1]=enable）
//! - 难度掩码 mask |= enable << difficulty_id（bit0=BASIC..bit4=ULTIMA）
//! - 输出 count(2B LE) + 每曲[id(2B LE) + mask(1B)]

use std::mem::transmute;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::OnceCell;

use crate::util::api::Api;
use crate::util::pattern;

/// 曲库加载 CALL 点签名：`PUSH EAX; CALL <load>; LEA ECX,[EBP-0x118]; CALL ...`
const MUSIC_LOAD_SIG: &str = "50 E8 ?? ?? ?? ?? 8D 8D E8 FE FF FF E8";

// MSVC std::_Tree_node 节点偏移
const NODE_PARENT: usize = 0x4;
const NODE_RIGHT: usize = 0x8;
const NODE_ISNIL: usize = 0xd;
const NODE_LEFT: usize = 0x0;
// MusicInfo（节点 _Myval）字段偏移
const MUSIC_ID: usize = 0x10;
const DIFF_VEC_FIRST: usize = 0x208;
const DIFF_VEC_LAST: usize = 0x20c;
const DIFF_STRIDE: usize = 0x40;
const DIFF_TYPE_ID: usize = 0x0;
const DIFF_ENABLE: usize = 0x1;

/// 红黑树遍历安全上限（曲库远小于此，纯防御野指针导致的死循环）
const MAX_NODES: usize = 100_000;

type LoadFn = unsafe extern "stdcall" fn(usize, usize, usize, usize, *const usize);

static API: OnceCell<Api> = OnceCell::new();
static TRAMPOLINE: OnceCell<usize> = OnceCell::new();
static MUSIC_PAYLOAD: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static MUSIC_CONTAINER: AtomicUsize = AtomicUsize::new(0);

pub fn init(api: &Api) {
    let text_base = api.text_base();
    let text_size = api.text_size();
    if text_base == 0 || text_size == 0 {
        api.log_warn("national match music: invalid text section");
        return;
    }

    let hit = pattern::scan_range(api, text_base, text_size, MUSIC_LOAD_SIG);
    if hit == 0 {
        api.log_warn("national match music: load signature not found");
        return;
    }

    // hit 指向 PUSH EAX；hit+1 是 E8，hit+2 是 rel32。
    let Some(rel) = read_i32(api, hit + 2) else {
        api.log_warn("national match music: failed to read call rel32");
        return;
    };
    // CALL 目标 = (E8 下一条指令地址) + rel = (hit + 1 + 5) + rel
    let mut load_fn = (hit + 6).wrapping_add(rel as usize);
    // 跳过 thunk：若目标是 E9 jmp，再解一跳。
    load_fn = resolve_thunk(api, load_fn);

    let _ = API.set(*api);

    let Some(trampoline) = api.hook_create(load_fn, music_load_detour as *const () as usize) else {
        api.log_warn("national match music: failed to create hook");
        return;
    };
    let _ = TRAMPOLINE.set(trampoline);

    if !api.hook_enable(load_fn) {
        api.log_warn("national match music: failed to enable hook");
        return;
    }

    api.log_info(&format!(
        "national match music: hooked load fn @ 0x{load_fn:08X}"
    ));
}

/// 取缓存的曲库帧 payload；未就绪时回退空列表（count=0）。
pub fn music_payload() -> Vec<u8> {
    MUSIC_PAYLOAD
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| vec![0, 0])
}

/// 收到服务端交集帧后改写游戏曲库：按 music id 匹配，逐难度把 enable 覆盖为交集掩码对应 bit。
/// 不在交集内的曲目保持原状（对齐 duolinguo）。
pub fn apply_intersection(payload: &[u8]) {
    let Some(api) = API.get() else {
        return;
    };
    let container = MUSIC_CONTAINER.load(Ordering::Acquire);
    if container == 0 {
        return;
    }

    let intersection = parse_intersection(payload);
    let count = intersection.len();
    match rewrite_enables(api, container, &intersection) {
        Some(applied) => api.log_info(&format!(
            "national match music: intersection {count} musics, rewrote {applied}"
        )),
        None => api.log_warn("national match music: intersection rewrite failed"),
    }
}

fn parse_intersection(payload: &[u8]) -> Vec<(u16, u8)> {
    if payload.len() < 2 {
        return Vec::new();
    }
    let count = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let off = 2 + i * 3;
        if off + 3 > payload.len() {
            break;
        }
        let id = u16::from_le_bytes([payload[off], payload[off + 1]]);
        out.push((id, payload[off + 2]));
    }
    out
}

fn rewrite_enables(api: &Api, container: usize, intersection: &[(u16, u8)]) -> Option<usize> {
    let head = read_usize(api, container)?;
    let mut node = read_usize(api, head + NODE_LEFT)?;

    let mut applied = 0usize;
    let mut guard = 0usize;
    while guard < MAX_NODES {
        guard += 1;
        if read_u8(api, node + NODE_ISNIL)? != 0 {
            break;
        }

        let id = read_u16(api, node + MUSIC_ID)?;
        if let Some(&(_, mask)) = intersection.iter().find(|&&(mid, _)| mid == id) {
            if rewrite_node_difficulties(api, node, mask).is_some() {
                applied += 1;
            }
        }

        node = inorder_successor(api, node, head)?;
        if node == head {
            break;
        }
    }
    Some(applied)
}

/// 对单曲的每个难度项写入 enable = (mask >> difficulty_id) & 1。
fn rewrite_node_difficulties(api: &Api, node: usize, mask: u8) -> Option<()> {
    let mut first = read_usize(api, node + DIFF_VEC_FIRST)?;
    let last = read_usize(api, node + DIFF_VEC_LAST)?;

    let mut guard = 0usize;
    while first != last && guard < 64 {
        guard += 1;
        let type_id = read_u8(api, first + DIFF_TYPE_ID)?;
        let enable = mask.wrapping_shr(u32::from(type_id) & 0x1f) & 1;
        api.mem_write(first + DIFF_ENABLE, &[enable]);
        first = first.wrapping_add(DIFF_STRIDE);
    }
    Some(())
}

unsafe extern "stdcall" fn music_load_detour(
    p1: usize,
    p2: usize,
    p3: usize,
    p4: usize,
    container: *const usize,
) {
    if let Some(&trampoline) = TRAMPOLINE.get() {
        let orig: LoadFn = transmute(trampoline);
        orig(p1, p2, p3, p4, container);
    }

    let Some(api) = API.get() else {
        return;
    };

    MUSIC_CONTAINER.store(container as usize, Ordering::Release);

    match dump_music(api, container) {
        Some(list) => {
            let payload = encode(&list);
            api.log_info(&format!(
                "national match music: loaded {} musics",
                list.len()
            ));
            if let Ok(mut guard) = MUSIC_PAYLOAD.lock() {
                *guard = Some(payload);
            }
        }
        None => api.log_warn("national match music: dump failed"),
    }
}

/// 中序遍历 std::map 红黑树，返回 (music_id, 难度掩码) 列表。
fn dump_music(api: &Api, container: *const usize) -> Option<Vec<(u16, u8)>> {
    if container.is_null() {
        return None;
    }
    // container -> _Myhead -> _Left（最小节点，中序起点）
    let head = read_usize(api, container as usize)?;
    let mut node = read_usize(api, head + NODE_LEFT)?;

    let mut out = Vec::new();
    let mut guard = 0usize;

    while guard < MAX_NODES {
        guard += 1;
        // _Isnil != 0 表示走到哨兵，结束。
        if read_u8(api, node + NODE_ISNIL)? != 0 {
            break;
        }

        if let (Some(id), Some(mask)) = (read_u16(api, node + MUSIC_ID), difficulty_mask(api, node))
        {
            out.push((id, mask));
        }

        node = inorder_successor(api, node, head)?;
        if node == head {
            break;
        }
    }

    Some(out)
}

/// 计算单曲难度掩码：遍历难度 vector，mask |= enable << difficulty_id。
fn difficulty_mask(api: &Api, node: usize) -> Option<u8> {
    let mut first = read_usize(api, node + DIFF_VEC_FIRST)?;
    let last = read_usize(api, node + DIFF_VEC_LAST)?;

    let mut mask = 0u8;
    let mut guard = 0usize;
    while first != last && guard < 64 {
        guard += 1;
        let type_id = read_u8(api, first + DIFF_TYPE_ID)?;
        let enable = read_u8(api, first + DIFF_ENABLE)?;
        mask |= enable.wrapping_shl(u32::from(type_id) & 0x1f);
        first = first.wrapping_add(DIFF_STRIDE);
    }
    Some(mask)
}

/// MSVC std::map 中序后继。
fn inorder_successor(api: &Api, node: usize, head: usize) -> Option<usize> {
    let right = read_usize(api, node + NODE_RIGHT)?;
    if read_u8(api, right + NODE_ISNIL)? == 0 {
        // 有右子树：取右子树最左节点。
        let mut cur = right;
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > MAX_NODES {
                return Some(head);
            }
            let left = read_usize(api, cur + NODE_LEFT)?;
            if read_u8(api, left + NODE_ISNIL)? != 0 {
                return Some(cur);
            }
            cur = left;
        }
    }

    // 无右子树：上溯到第一个「自己是左子」的祖先。
    let mut cur = node;
    let mut parent = read_usize(api, node + NODE_PARENT)?;
    let mut guard = 0usize;
    while read_u8(api, parent + NODE_ISNIL)? == 0 && cur == read_usize(api, parent + NODE_RIGHT)? {
        guard += 1;
        if guard > MAX_NODES {
            return Some(head);
        }
        cur = parent;
        parent = read_usize(api, parent + NODE_PARENT)?;
    }
    Some(parent)
}

/// 编码为 reflector Music 帧 payload：count(2B LE) + 每曲[id(2B LE) + mask(1B)]。
fn encode(list: &[(u16, u8)]) -> Vec<u8> {
    let count = u16::try_from(list.len()).unwrap_or(u16::MAX);
    let mut buf = Vec::with_capacity(2 + list.len() * 3);
    buf.extend_from_slice(&count.to_le_bytes());
    for &(id, mask) in list.iter().take(count as usize) {
        buf.extend_from_slice(&id.to_le_bytes());
        buf.push(mask);
    }
    buf
}

/// 若地址处是 E9 相对 jmp（编译器 thunk），解析其目标；否则原样返回。
fn resolve_thunk(api: &Api, addr: usize) -> usize {
    let mut op = [0u8; 1];
    if api.mem_read(addr, &mut op) && op[0] == 0xE9 {
        if let Some(rel) = read_i32(api, addr + 1) {
            return (addr + 5).wrapping_add(rel as usize);
        }
    }
    addr
}

fn read_i32(api: &Api, addr: usize) -> Option<i32> {
    let mut buf = [0u8; 4];
    api.mem_read(addr, &mut buf)
        .then(|| i32::from_le_bytes(buf))
}

fn read_usize(api: &Api, addr: usize) -> Option<usize> {
    let mut buf = [0u8; 4];
    api.mem_read(addr, &mut buf)
        .then(|| u32::from_le_bytes(buf) as usize)
}

fn read_u16(api: &Api, addr: usize) -> Option<u16> {
    let mut buf = [0u8; 2];
    api.mem_read(addr, &mut buf)
        .then(|| u16::from_le_bytes(buf))
}

fn read_u8(api: &Api, addr: usize) -> Option<u8> {
    let mut buf = [0u8; 1];
    api.mem_read(addr, &mut buf).then(|| buf[0])
}
