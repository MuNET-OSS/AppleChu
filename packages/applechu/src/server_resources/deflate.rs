//! 解压：调用游戏自带的 zlib（`gm::Deflate` / `projClient::Deflate`）
//!
//! 资源包若使用 deflate 压缩，直接复用游戏内嵌的 zlib 1.2.8，不自写 inflate。
//! 挂钩点选用游戏自己的高层封装 `sub_9AB850`（RVA `0x5AB850`）：它按 `__thiscall` 接收
//! `(this, 输入 std::string*, 输出 std::string*)`，内部完成初始化、inflate
//! 与结果赋值，因此不需要我们手工维护 `z_stream`。
//!
//! `this` 需要一个带虚表的对象（布局 `[vtable][z_stream]`，共 0x3C 字节），
//! 虚表指针取自 IDA 里的 `off_18EFDBC`。

use crate::util::pattern;

/// `gm::Deflate` 的虚表（IDA: `off_18EFDBC`）
const DEFLATE_VTABLE_RVA: usize = 0x14E_FDBC;
/// 高层解压封装的函数头特征（`sub_9AB850`）
const INFLATE_SIGNATURE: &str = concat!(
    "55 8D AC 24 E4 DF FC FF B8 1C 20 03 00 E8 ?? ?? ?? ?? ",
    "6A FF 68 ?? ?? ?? ?? 64 A1 00 00 00 00 50"
);

/// 对象大小：`[vtable]` + `z_stream`(0x38)
const DEFLATE_OBJECT_SIZE: usize = 0x3C;
/// MSVC `std::string` 对象大小
const STD_STRING_SIZE: usize = 0x18;
/// `_Myres` 达到该值表示数据在堆上
const SSO_CAPACITY: u32 = 16;

type InflateFn = unsafe extern "thiscall" fn(*mut u8, *mut u8, *mut u8) -> i32;

/// 用游戏自带的 zlib 解压 `input`
pub fn inflate(input: &[u8]) -> Result<Vec<u8>, String> {
    let Some(api) = crate::util::api::API.get() else {
        return Err("api handle is not available".to_owned());
    };
    let base = api.game_base();
    if base == 0 {
        return Err("game base is not available".to_owned());
    }

    let entry = pattern::scan(api, INFLATE_SIGNATURE);
    if entry == 0 {
        return Err("deflate entry signature not found".to_owned());
    }
    let vtable = base
        .checked_add(DEFLATE_VTABLE_RVA)
        .ok_or_else(|| "deflate vtable address overflowed".to_owned())?;

    // 对象与两个 std::string 都放在本线程栈上；z_stream 由被调函数负责清零
    let mut object = [0u8; DEFLATE_OBJECT_SIZE];
    object[..4].copy_from_slice(&(vtable as u32).to_le_bytes());

    let mut input_buffer = [0u8; STD_STRING_SIZE];
    let mut output_buffer = [0u8; STD_STRING_SIZE];
    unsafe {
        std::ptr::write_bytes(output_buffer.as_mut_ptr(), 0, STD_STRING_SIZE);
        layout_string(&mut input_buffer, input)?;
        // 空输出的 `_Myres` 置 0，避免被调函数在赋值前尝试释放栈内存
        output_buffer[0x14..0x18].copy_from_slice(&0u32.to_le_bytes());
    }

    let inflate: InflateFn = unsafe { std::mem::transmute(entry) };
    let status = unsafe {
        inflate(
            object.as_mut_ptr(),
            input_buffer.as_mut_ptr(),
            output_buffer.as_mut_ptr(),
        )
    };

    // 读出解压结果
    let result = unsafe { read_string(&output_buffer) }.ok_or_else(|| {
        format!("deflate returned {status} without a usable output buffer")
    })?;

    // 输出若落在堆上，用游戏的释放函数回收，避免每次解压泄漏
    unsafe {
        let capacity = u32::from_le_bytes(output_buffer[0x14..0x18].try_into().unwrap());
        if capacity >= SSO_CAPACITY {
            let pointer = u32::from_le_bytes(output_buffer[0..4].try_into().unwrap()) as usize;
            deallocate(pointer, capacity as usize + 1);
        }
    }

    if result.is_empty() && status != 1 {
        return Err(format!("deflate failed with status {status}"));
    }
    Ok(result)
}

/// 在调用方缓冲区内布置一个指向 `data` 的 MSVC `std::string`
///
/// 使用「堆视图」布局（`_Myres >= 16`），让被调函数直接读 `_Bx._Ptr`；
/// 这里只借用 `data` 的指针，不负责其生命周期。
unsafe fn layout_string(buffer: &mut [u8; STD_STRING_SIZE], data: &[u8]) -> Result<(), String> {
    let length = u32::try_from(data.len()).map_err(|_| "input is too large".to_owned())?;
    buffer[0..4].copy_from_slice(&(data.as_ptr() as u32).to_le_bytes());
    buffer[0x10..0x14].copy_from_slice(&length.to_le_bytes());
    // 置为 SSO 上限以上的值，确保走指针路径且不被误判为可释放
    buffer[0x14..0x18].copy_from_slice(&u32::MAX.to_le_bytes());
    Ok(())
}

/// 读取 MSVC `std::string` 内容
unsafe fn read_string(buffer: &[u8; STD_STRING_SIZE]) -> Option<Vec<u8>> {
    let length = u32::from_le_bytes(buffer[0x10..0x14].try_into().ok()?) as usize;
    if length == 0 {
        return Some(Vec::new());
    }
    if length > 1 << 26 {
        return None;
    }
    let capacity = u32::from_le_bytes(buffer[0x14..0x18].try_into().ok()?);
    let data = if capacity >= SSO_CAPACITY {
        u32::from_le_bytes(buffer[0..4].try_into().ok()?) as usize as *const u8
    } else {
        buffer.as_ptr()
    };
    if data.is_null() {
        return None;
    }
    Some(std::slice::from_raw_parts(data, length).to_vec())
}

/// 释放游戏内嵌 zlib 分配的字符串缓冲（`std::string::_Deallocate`，`sub_448A31`）
unsafe fn deallocate(pointer: usize, size: usize) {
    if pointer == 0 {
        return;
    }
    let Some(api) = crate::util::api::API.get() else {
        return;
    };
    let base = api.game_base();
    if base == 0 {
        return;
    }
    let Some(free) = base.checked_add(0x48A31) else {
        return;
    };
    let free: unsafe extern "cdecl" fn(usize, usize, usize) = std::mem::transmute(free);
    free(pointer, size, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_heap_backed_string_view() {
        // Given: 一段输入数据。
        let data = b"hello";
        let mut buffer = [0u8; STD_STRING_SIZE];

        // When: 布置成 std::string 视图。
        unsafe { layout_string(&mut buffer, data).unwrap() };

        // Then: 长度与指针写入正确，容量足以让被调函数走指针路径。
        assert_eq!(u32::from_le_bytes(buffer[0x10..0x14].try_into().unwrap()), 5);
        assert!(u32::from_le_bytes(buffer[0x14..0x18].try_into().unwrap()) >= SSO_CAPACITY);
        assert_eq!(
            u32::from_le_bytes(buffer[0..4].try_into().unwrap()) as usize,
            data.as_ptr() as usize
        );
    }

    #[test]
    fn reads_inline_and_heap_strings() {
        // Given: 一个内联存储的空字符串。
        let mut inline = [0u8; STD_STRING_SIZE];
        inline[0x14..0x18].copy_from_slice(&15u32.to_le_bytes());

        // When/Then: 空长度读出空内容。
        assert_eq!(unsafe { read_string(&inline) }, Some(Vec::new()));
    }
}
