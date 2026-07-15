use std::fs;

use super::log::log_info;

/// 打包崩溃诊断 zip：崩溃日志 + loader 日志 + 配置 + 启动脚本 + mods/bin 文件清单
/// 返回 zip 路径（失败返回 None）
pub fn build(base_dir: &str, crash_dir: &str, stamp: &str, crash_log: &str) -> Option<String> {
    if base_dir.is_empty() {
        return None;
    }
    let zip_path = format!("{crash_dir}\\crash_report_{stamp}.zip");
    let mut zip = ZipWriter::new();

    add_file(&mut zip, crash_log, "crash.log");
    add_file(
        &mut zip,
        &format!("{base_dir}\\chumod_loader.log"),
        "chumod_loader.log",
    );
    add_file(
        &mut zip,
        &format!("{base_dir}\\AppleChu.toml"),
        "AppleChu.toml",
    );
    add_file(
        &mut zip,
        &format!("{base_dir}\\segatools.ini"),
        "segatools.ini",
    );
    add_file(&mut zip, &format!("{base_dir}\\start.bat"), "start.bat");
    add_file(
        &mut zip,
        &format!("{base_dir}\\DEVICE\\aime.txt"),
        "aime.txt",
    );

    let mods_listing = list_dir_with_hash(&format!("{base_dir}\\mods"));
    zip.add("mods_listing.txt", mods_listing.as_bytes());
    let bin_listing = list_dir_with_hash(base_dir);
    zip.add("bin_listing.txt", bin_listing.as_bytes());

    let bytes = zip.finish();
    match fs::write(&zip_path, &bytes) {
        Ok(_) => {
            log_info(&format!("crash report zipped: {zip_path}"));
            Some(zip_path)
        }
        Err(_) => None,
    }
}

fn add_file(zip: &mut ZipWriter, path: &str, name_in_zip: &str) {
    if let Ok(data) = fs::read(path) {
        zip.add(name_in_zip, &data);
    }
}

fn list_dir_with_hash(dir: &str) -> String {
    let mut out = String::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let hash = fs::read(&path).map(|d| fnv1a(&d)).unwrap_or(0);
            out.push_str(&format!(
                "{}\t{} bytes\t{:016x}\n",
                path.display(),
                size,
                hash
            ));
        } else if path.is_dir() {
            out.push_str(&format!("{}\\\n", path.display()));
        }
    }
    out
}

/// FNV-1a 64-bit：轻量文件指纹（核对版本/篡改，非加密用途）
fn fnv1a(data: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 最简 ZIP 写入器（store 模式，无压缩），避免引入 zip crate 依赖
struct ZipWriter {
    buffer: Vec<u8>,
    entries: Vec<CentralEntry>,
}

struct CentralEntry {
    name: String,
    crc: u32,
    size: u32,
    offset: u32,
}

impl ZipWriter {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            entries: Vec::new(),
        }
    }

    fn add(&mut self, name: &str, data: &[u8]) {
        let offset = self.buffer.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;

        // Local file header (signature 0x04034b50)
        self.buffer.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.buffer.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // flags
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // method = store
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // mod time
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // mod date
        self.buffer.extend_from_slice(&crc.to_le_bytes());
        self.buffer.extend_from_slice(&size.to_le_bytes()); // compressed size
        self.buffer.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        self.buffer
            .extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // extra len
        self.buffer.extend_from_slice(name.as_bytes());
        self.buffer.extend_from_slice(data);

        self.entries.push(CentralEntry {
            name: name.to_owned(),
            crc,
            size,
            offset,
        });
    }

    fn finish(mut self) -> Vec<u8> {
        let cd_start = self.buffer.len() as u32;
        for entry in &self.entries {
            self.buffer.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central dir signature
            self.buffer.extend_from_slice(&20u16.to_le_bytes()); // version made by
            self.buffer.extend_from_slice(&20u16.to_le_bytes()); // version needed
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // flags
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // method
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // mod time
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // mod date
            self.buffer.extend_from_slice(&entry.crc.to_le_bytes());
            self.buffer.extend_from_slice(&entry.size.to_le_bytes());
            self.buffer.extend_from_slice(&entry.size.to_le_bytes());
            self.buffer
                .extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // extra
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // comment
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // disk number
            self.buffer.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            self.buffer.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            self.buffer.extend_from_slice(&entry.offset.to_le_bytes());
            self.buffer.extend_from_slice(entry.name.as_bytes());
        }
        let cd_size = self.buffer.len() as u32 - cd_start;

        // End of central directory (signature 0x06054b50)
        self.buffer.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // disk
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // cd disk
        self.buffer
            .extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        self.buffer
            .extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        self.buffer.extend_from_slice(&cd_size.to_le_bytes());
        self.buffer.extend_from_slice(&cd_start.to_le_bytes());
        self.buffer.extend_from_slice(&0u16.to_le_bytes()); // comment len
        self.buffer
    }
}

/// CRC-32 (ZIP 标准多项式 0xEDB88320)，ZIP 格式必需的校验字段
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
