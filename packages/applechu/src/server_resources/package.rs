//! Option 资源包（zip 容器）解析与虚拟目录树
//!
//! 服务器下发的资源包是一个 zip，内容为单个 `AXXX` 目录树（与磁盘 option
//! 同构，含 `data.conf`）。本模块把它解析为内存中的文件表，并提供目录枚举
//! 与文件读取，供 VFS 层呈现给游戏。
//!
//! 支持 zip 的 `store`（method 0）与 `deflate`（method 8）。deflate 需要
//! 游戏自带的 `projClient::Deflate`（RTTI 0x1C5CC34），未接线前对 deflate
//! 条目返回明确错误。

use std::collections::BTreeMap;

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;
/// zip 注释最长 65535 字节，EOCD 结构 22 字节
const EOCD_SEARCH_LIMIT: usize = 22 + 0xffff;

/// 内存中的 Option 资源包
pub struct OptionPackage {
    /// 以 `/` 分隔的相对路径 → 文件内容
    files: BTreeMap<String, Vec<u8>>,
    /// 根目录名（`AXXX`）
    root: String,
}

impl OptionPackage {
    /// 解析 zip 字节流
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let central = locate_central_directory(bytes)?;
        let mut files = BTreeMap::new();
        let mut root: Option<String> = None;
        let mut cursor = central;
        let mut pending = 0usize;

        loop {
            let signature = read_u32(bytes, cursor)
                .ok_or_else(|| "zip central directory is truncated".to_owned())?;
            if signature != CENTRAL_SIGNATURE {
                break;
            }
            let method = read_u16(bytes, cursor + 10)
                .ok_or_else(|| "zip central entry is truncated".to_owned())?;
            let compressed_size = read_u32(bytes, cursor + 20)
                .ok_or_else(|| "zip central entry is truncated".to_owned())?
                as usize;
            let name_len = read_u16(bytes, cursor + 28)
                .ok_or_else(|| "zip central entry is truncated".to_owned())?
                as usize;
            let extra_len = read_u16(bytes, cursor + 30)
                .ok_or_else(|| "zip central entry is truncated".to_owned())?
                as usize;
            let comment_len = read_u16(bytes, cursor + 32)
                .ok_or_else(|| "zip central entry is truncated".to_owned())?
                as usize;
            let local_offset = read_u32(bytes, cursor + 42)
                .ok_or_else(|| "zip central entry is truncated".to_owned())?
                as usize;

            let name_start = cursor + 46;
            let name_bytes = bytes
                .get(name_start..name_start + name_len)
                .ok_or_else(|| "zip entry name is truncated".to_owned())?;
            let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");

            // 目录条目以 `/` 结尾，不需要建立文件
            if !name.ends_with('/') && !name.is_empty() {
                let data = read_entry_data(bytes, local_offset, method, compressed_size)?;
                if let Some(segment) = name.split('/').next() {
                    if !segment.is_empty() {
                        match &root {
                            Some(existing) if existing != segment => {
                                return Err(format!(
                                    "option package spans multiple roots: {existing} and {segment}"
                                ));
                            }
                            _ => root = Some(segment.to_owned()),
                        }
                    }
                }
                files.insert(name, data);
            }

            pending += 1;
            if pending > 10_000 {
                return Err("option package has too many entries".to_owned());
            }
            cursor = name_start
                .checked_add(name_len)
                .and_then(|value| value.checked_add(extra_len))
                .and_then(|value| value.checked_add(comment_len))
                .ok_or_else(|| "zip central directory offset overflowed".to_owned())?;
        }

        if files.is_empty() {
            return Err("option package contains no files".to_owned());
        }
        let root = root.ok_or_else(|| "option package has no root directory".to_owned())?;

        Ok(Self { files, root })
    }

    /// 根目录名（`AXXX`）
    pub fn root(&self) -> &str {
        &self.root
    }

    /// 读取文件；路径以 `/` 分隔
    pub fn read_file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(&normalize(path)).map(Vec::as_slice)
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(&normalize(path))
    }

    /// 列出目录下的直接子项（名称, 是否目录），按名称排序
    pub fn list_dir(&self, path: &str) -> Vec<(String, bool)> {
        let prefix = {
            let normalized = normalize(path);
            if normalized.is_empty() {
                String::new()
            } else {
                format!("{normalized}/")
            }
        };

        let mut seen: BTreeMap<String, bool> = BTreeMap::new();
        for name in self.files.keys() {
            let Some(rest) = name.strip_prefix(&prefix) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            match rest.split_once('/') {
                Some((segment, _)) => {
                    seen.insert(segment.to_owned(), true);
                }
                None => {
                    seen.insert(rest.to_owned(), false);
                }
            }
        }
        seen.into_iter().collect()
    }

    /// 全部文件路径，用于日志诊断
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

fn normalize(path: &str) -> String {
    path.trim_matches('/').replace('\\', "/")
}

/// 定位中央目录起始偏移
fn locate_central_directory(bytes: &[u8]) -> Result<usize, String> {
    // EOCD 结构本身 22 字节，短于它不可能是 zip
    if bytes.len() < 22 {
        return Err("zip is too short".to_owned());
    }
    let search_start = bytes.len().saturating_sub(EOCD_SEARCH_LIMIT);
    let mut index = bytes.len() - 22;
    loop {
        if read_u32(bytes, index) == Some(EOCD_SIGNATURE) {
            let offset = read_u32(bytes, index + 16)
                .ok_or_else(|| "zip end-of-central-directory is truncated".to_owned())?
                as usize;
            if offset < bytes.len() {
                return Ok(offset);
            }
        }
        if index == search_start {
            return Err("zip end-of-central-directory not found".to_owned());
        }
        index -= 1;
    }
}

/// 读取单个条目的数据（store 直接切片，deflate 暂不支持）
fn read_entry_data(
    bytes: &[u8],
    local_offset: usize,
    method: u16,
    compressed_size: usize,
) -> Result<Vec<u8>, String> {
    // 本地文件头：签名(4) + 版本(2) + 标志(2) + 方法(2) + 时间(2) + 日期(2)
    //            + CRC(4) + 压缩大小(4) + 原始大小(4) + 名称长度(2) + 扩展长度(2)
    let name_len = read_u16(bytes, local_offset + 26)
        .ok_or_else(|| "zip local header is truncated".to_owned())?
        as usize;
    let extra_len = read_u16(bytes, local_offset + 28)
        .ok_or_else(|| "zip local header is truncated".to_owned())?
        as usize;
    let data_start = local_offset
        .checked_add(30)
        .and_then(|value| value.checked_add(name_len))
        .and_then(|value| value.checked_add(extra_len))
        .ok_or_else(|| "zip local header offset overflowed".to_owned())?;
    let data = bytes
        .get(data_start..data_start + compressed_size)
        .ok_or_else(|| "zip entry data is truncated".to_owned())?;

    match method {
        METHOD_STORE => Ok(data.to_vec()),
        METHOD_DEFLATE => Err(
            "option package uses deflate compression, which is not wired up yet".to_owned(),
        ),
        other => Err(format!("unsupported zip compression method {other}")),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// 测试用的最小 zip 构造器（store 模式），与 `crash_zip` 的写出逻辑同源
#[cfg(test)]
pub fn build_store_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut entries = Vec::new();
    for (name, data) in files {
        let offset = buffer.len() as u32;
        buffer.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        buffer.extend_from_slice(&20u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buffer.extend_from_slice(&(name.len() as u16).to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(name.as_bytes());
        buffer.extend_from_slice(data);
        entries.push((name.to_string(), data.len() as u32, offset));
    }
    let central_start = buffer.len() as u32;
    for (name, size, offset) in &entries {
        buffer.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
        buffer.extend_from_slice(&20u16.to_le_bytes());
        buffer.extend_from_slice(&20u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer.extend_from_slice(&size.to_le_bytes());
        buffer.extend_from_slice(&size.to_le_bytes());
        buffer.extend_from_slice(&(name.len() as u16).to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u16.to_le_bytes());
        buffer.extend_from_slice(&0u32.to_le_bytes());
        buffer.extend_from_slice(&offset.to_le_bytes());
        buffer.extend_from_slice(name.as_bytes());
    }
    let central_size = buffer.len() as u32 - central_start;
    buffer.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    buffer.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    buffer.extend_from_slice(&central_size.to_le_bytes());
    buffer.extend_from_slice(&central_start.to_le_bytes());
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        build_store_zip(&[
            ("A200/data.conf", b"[Version]\nName = CHUNITHM\n"),
            ("A200/music/music9001/Music.xml", b"<Music/>"),
            ("A200/music/music9001/9001_03.c2s", b"\x01\x02\x03"),
            ("A200/releaseTag/releaseTag000099/ReleaseTag.xml", b"<Tag/>"),
        ])
    }

    #[test]
    fn parses_root_and_files() {
        // Given: 一个单根 Option 包。
        let package = OptionPackage::parse(&sample()).expect("应解析成功");

        // Then: 根目录与文件内容正确。
        assert_eq!(package.root(), "A200");
        assert_eq!(package.file_count(), 4);
        assert_eq!(
            package.read_file("A200/data.conf"),
            Some(&b"[Version]\nName = CHUNITHM\n"[..])
        );
    }

    #[test]
    fn lists_directories_not_deep_files() {
        // Given: 已解析的 Option 包。
        let package = OptionPackage::parse(&sample()).unwrap();

        // When: 枚举包根与 `A200` 目录。
        let root = package.list_dir("");
        let inner = package.list_dir("A200");

        // Then: 根下只有 AXXX 目录；其内只返回直接子项，目录被标记。
        assert_eq!(root, vec![("A200".to_owned(), true)]);
        assert_eq!(
            inner,
            vec![
                ("data.conf".to_owned(), false),
                ("music".to_owned(), true),
                ("releaseTag".to_owned(), true),
            ]
        );
    }

    #[test]
    fn rejects_packages_spanning_multiple_roots() {
        // Given: 含两个 AXXX 根的包。
        let bytes = build_store_zip(&[("A200/a.txt", b"a"), ("A201/b.txt", b"b")]);

        // Then: 解析失败，避免挂载出无法归属的内容。
        assert!(OptionPackage::parse(&bytes).is_err());
    }

    #[test]
    fn rejects_truncated_input() {
        // Given: 被截断的 zip。
        let bytes = sample();
        let truncated = &bytes[..bytes.len() / 2];

        // Then: 解析失败而不是 panic。
        assert!(OptionPackage::parse(truncated).is_err());
        assert!(OptionPackage::parse(b"").is_err());
    }
}
