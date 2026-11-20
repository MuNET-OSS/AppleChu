//! Option 资源包（zip 容器）解析与解压
//!
//! 服务器下发的资源包是一个 zip，内容为单个 `AXXX` 目录树（与磁盘 option
//! 同构，`data.conf` 可有可无）。本模块把它解析为文件表，再**解压到缓存目录**：
//! 挂载与读取都交给 `platform/vfs` 与游戏本身，内存里不长期保留内容。
//!
//! 支持 zip 的 `store`（method 0）与 `deflate`（method 8）。deflate 复用游戏
//! 自带的 zlib（`projClient::Deflate`，RTTI 0x1C5CC34）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
const METHOD_STORE: u16 = 0;
const METHOD_DEFLATE: u16 = 8;
/// zip 注释最长 65535 字节，EOCD 结构 22 字节
const EOCD_SEARCH_LIMIT: usize = 22 + 0xffff;

/// 内存中的 Option 资源包
pub struct OptionPackage {
    /// **小写**路径（`/` 分隔）→ 条目；Windows 路径不区分大小写，
    /// 包内 `a189/data.conf` 与 `A189/data.conf` 必须视为同一条
    files: BTreeMap<String, Entry>,
    /// 根目录名（`AXXX`，保留原始大小写）
    root: String,
}

/// 单个文件条目
struct Entry {
    /// 原始大小写路径（`/` 分隔）
    name: String,
    data: Vec<u8>,
}

impl OptionPackage {
    /// 解析 zip 字节流
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let central = locate_central_directory(bytes)?;
        let mut files: BTreeMap<String, Entry> = BTreeMap::new();
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
                // 解压前就挡住路径穿越，避免写出缓存目录之外
                let key = safe_relative(&name)?;
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
                files.insert(key, Entry { name, data });
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

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// 把包内文件全部解到 `directory` 下，返回写出数量
    ///
    /// 调用方负责先清干净目标目录，避免上一次的包留下孤儿文件。
    pub fn extract_to(&self, directory: &Path) -> Result<usize, String> {
        let mut written = 0usize;
        for entry in self.files.values() {
            let relative = relative_path(&entry.name)?;
            let target = directory.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
            }
            std::fs::write(&target, &entry.data)
                .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
            written += 1;
        }
        Ok(written)
    }
}

/// 校验并归一化条目路径，返回 zip 内相对路径（`/` 分隔、小写）
///
/// 拒绝绝对路径、盘符与 `..`，避免 zip 条目写出解压目录之外。
fn safe_relative(name: &str) -> Result<String, String> {
    let mut normalized = String::new();
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(format!("option package entry has an unsafe path: {name}"));
        }
        if segment.contains(':') || segment.contains('\\') {
            return Err(format!("option package entry has an unsafe path: {name}"));
        }
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(segment);
    }
    if normalized.is_empty() {
        return Err("option package has an empty entry name".to_owned());
    }
    Ok(normalized.to_ascii_lowercase())
}

/// 条目路径 → 可直接 `join` 到解压目录的相对路径
fn relative_path(name: &str) -> Result<PathBuf, String> {
    safe_relative(name)?;
    let mut path = PathBuf::new();
    for segment in name.split('/') {
        path.push(segment);
    }
    Ok(path)
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

/// 读取单个条目的数据（store 直接切片，deflate 交给游戏内嵌 zlib）
fn read_entry_data(
    bytes: &[u8],
    local_offset: usize,
    method: u16,
    compressed_size: usize,
) -> Result<Vec<u8>, String> {
    // 本地文件头：签名(4) + 版本(2) + 标志(2) + 方法(2) + 时间(2) + 日期(2)
    //            + CRC(4) + 压缩大小(4) + 原始大小(4) + 名称长度(2) + 扩展长度(2)
    let name_len = read_u16(bytes, local_offset + 26)
        .ok_or_else(|| "zip local header is truncated".to_owned())? as usize;
    let extra_len = read_u16(bytes, local_offset + 28)
        .ok_or_else(|| "zip local header is truncated".to_owned())? as usize;
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
        METHOD_DEFLATE => crate::server_resources::deflate::inflate(data),
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

/// 测试用的最小 zip 构造器（store 模式）
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

        // Then: 根目录与文件数量正确。
        assert_eq!(package.root(), "A200");
        assert_eq!(package.file_count(), 4);
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

    #[test]
    fn rejects_entries_escaping_the_extraction_root() {
        // Given: 试图穿越目录的 zip 条目。
        // Then: 解析即失败，不会写出解压目录之外。
        assert!(OptionPackage::parse(&build_store_zip(&[("A200/../../evil.txt", b"x")])).is_err());
        assert!(OptionPackage::parse(&build_store_zip(&[("C:/evil.txt", b"x")])).is_err());
        assert!(OptionPackage::parse(&build_store_zip(&[("/abs.txt", b"x")])).is_err());
    }

    #[test]
    fn extracts_the_tree_under_the_target_directory() {
        // Given: 一个含嵌套目录的单根包。
        let package = OptionPackage::parse(&sample()).unwrap();
        let directory =
            std::env::temp_dir().join(format!("applechu-option-extract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        // When: 解压到目标目录。
        let written = package.extract_to(&directory).expect("解压应成功");

        // Then: 文件按包内层级落地，内容一致。
        assert_eq!(written, 4);
        assert_eq!(
            std::fs::read(directory.join("A200/music/music9001/9001_03.c2s")).unwrap(),
            b"\x01\x02\x03"
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn entry_paths_are_lowercased_for_lookup() {
        // Given: 大小写不一致的条目路径。
        // Then: 归一化后一致，且共享同一个 map key（后者覆盖前者）。
        assert_eq!(safe_relative("A200/Data.Conf").unwrap(), "a200/data.conf");
        assert_eq!(safe_relative("a200/data.conf").unwrap(), "a200/data.conf");
    }
}
