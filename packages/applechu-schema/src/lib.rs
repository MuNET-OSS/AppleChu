mod example;
mod generator;

use sha2::{Digest, Sha256};
use std::fmt;

pub use example::{DEFAULT_CONFIG_HEADER, EXAMPLE_CONFIG_FILE};
pub use generator::generate_from_rust_dir;

const MAGIC: &[u8; 8] = b"ACMANI\0\0";
pub const CONTAINER_VERSION: u16 = 1;
pub const HEADER_LENGTH: u16 = 64;

pub fn canonical_key(key: &str) -> String {
    let mut output = String::with_capacity(key.len());
    let mut capitalize = true;
    for character in key.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            output.extend(character.to_uppercase());
            capitalize = false;
        } else {
            output.push(character);
        }
    }
    output
}

pub fn keys_equal(left: &str, right: &str) -> bool {
    left.bytes()
        .filter(|byte| *byte != b'_')
        .map(|byte| byte.to_ascii_lowercase())
        .eq(right
            .bytes()
            .filter(|byte| *byte != b'_')
            .map(|byte| byte.to_ascii_lowercase()))
}

#[derive(Clone, Debug)]
pub struct OptionSpec {
    pub value: toml::Value,
    pub label: LocalizedText,
}

#[derive(Clone, Debug)]
pub struct EntrySpec {
    pub key: String,
    pub value_type: String,
    pub default: Option<toml::Value>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub emit_default: bool,
    pub emit_comment: bool,
    pub hidden: bool,
    pub options: Vec<OptionSpec>,
    pub comment: Option<LocalizedText>,
    pub description: Option<LocalizedText>,
}

#[derive(Clone, Debug, Default)]
pub struct LocalizedText {
    pub zh: Option<String>,
    pub en: Option<String>,
}

impl LocalizedText {
    fn from_value(value: Option<&toml::Value>) -> Option<Self> {
        let table = value?.as_table()?;
        Some(Self {
            zh: table
                .get("zh")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            en: table
                .get("en")
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
        })
    }

    pub fn zh_or_en(&self) -> Option<&str> {
        self.zh.as_deref().or(self.en.as_deref())
    }
}

#[derive(Clone, Debug)]
pub struct SectionSpec {
    pub id: String,
    pub order: usize,
    pub default_on: bool,
    pub always_enabled: bool,
    pub community: bool,
    pub hidden: bool,
    pub label: LocalizedText,
    pub description: Option<LocalizedText>,
    pub entries: Vec<EntrySpec>,
}

#[derive(Clone, Debug)]
pub struct Schema {
    source: String,
    document: toml::Value,
    sections: Vec<SectionSpec>,
}

#[derive(Clone, Copy, Debug)]
pub struct Acmani<'a> {
    pub manifest: &'a str,
    pub default_config: &'a str,
}

#[derive(Debug)]
pub enum SchemaError {
    Toml(toml::de::Error),
    Serialize(toml::ser::Error),
    Invalid(String),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(error) => write!(f, "schema TOML 解析失败: {error}"),
            Self::Serialize(error) => write!(f, "schema TOML 序列化失败: {error}"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<toml::de::Error> for SchemaError {
    fn from(error: toml::de::Error) -> Self {
        Self::Toml(error)
    }
}

impl From<toml::ser::Error> for SchemaError {
    fn from(error: toml::ser::Error) -> Self {
        Self::Serialize(error)
    }
}

impl Schema {
    pub fn parse(source: impl Into<String>) -> Result<Self, SchemaError> {
        let source = source.into();
        let mut document = source.parse::<toml::Value>()?;
        let root = document
            .as_table()
            .ok_or_else(|| SchemaError::Invalid("schema 根必须是 TOML 表".to_owned()))?;
        let config = root
            .get("config")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| SchemaError::Invalid("schema 缺少 [config]".to_owned()))?;
        let raw_sections = config
            .get("sections")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| SchemaError::Invalid("schema 缺少 config.sections".to_owned()))?;

        let mut sections = Vec::with_capacity(raw_sections.len());
        for (index, raw) in raw_sections.iter().enumerate() {
            let table = raw.as_table().ok_or_else(|| {
                SchemaError::Invalid(format!("config.sections[{index}] 必须是表"))
            })?;
            let id = table
                .get("id")
                .and_then(toml::Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| SchemaError::Invalid(format!("config.sections[{index}] 缺少 id")))?;
            let mut always_values = Vec::new();
            for key in ["always_enabled", "alwaysEnabled"] {
                if let Some(value) = bool_field(table, key)? {
                    always_values.push((key, value));
                }
            }
            if always_values
                .windows(2)
                .any(|values| values[0].1 != values[1].1)
            {
                return Err(SchemaError::Invalid(format!(
                    "配置栏目 {id} 的 always_enabled 元数据存在冲突"
                )));
            }
            let always_enabled = always_values
                .first()
                .map(|(_, value)| *value)
                .unwrap_or(false);
            // `default_enabled` 是 manifest 的兼容字段名
            // 同时接受 `default_on` 和 `defaultOn` 并统一为运行时语义
            let mut default_values = Vec::new();
            for key in ["default_on", "defaultOn", "default_enabled"] {
                if let Some(value) = bool_field(table, key)? {
                    default_values.push((key, value));
                }
            }
            if default_values
                .windows(2)
                .any(|values| values[0].1 != values[1].1)
            {
                return Err(SchemaError::Invalid(format!(
                    "配置栏目 {id} 的 default_on 元数据存在冲突"
                )));
            }
            let default_on = default_values
                .first()
                .map(|(_, value)| *value)
                .unwrap_or(always_enabled);
            let hidden = table
                .get("hidden")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false);
            let mut entries = parse_entries(table, id)?;
            for entry in &mut entries {
                entry.key = canonical_key(&entry.key);
            }
            if !hidden
                && !always_enabled
                && !entries.iter().any(|entry| keys_equal(&entry.key, "enable"))
            {
                entries.insert(0, enable_entry(default_on));
            }
            sections.push(SectionSpec {
                id: id.to_owned(),
                order: index,
                default_on,
                always_enabled,
                community: bool_field(table, "community")?.unwrap_or(false),
                hidden,
                label: LocalizedText::from_value(table.get("label")).unwrap_or_default(),
                description: LocalizedText::from_value(table.get("description")),
                entries,
            });
        }
        validate_sections(&sections)?;
        validate_groups(root, &sections)?;
        inject_enable_entries(&mut document, &sections)?;
        canonicalize_document_keys(&mut document)?;
        Ok(Self {
            source,
            document,
            sections,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn sections(&self) -> &[SectionSpec] {
        &self.sections
    }

    pub fn section(&self, id: &str) -> Option<&SectionSpec> {
        self.sections
            .iter()
            .find(|section| keys_equal(&section.id, id))
    }

    pub fn entry(&self, section: &str, key: &str) -> Option<&EntrySpec> {
        self.section(section).and_then(|section| {
            section
                .entries
                .iter()
                .find(|entry| keys_equal(&entry.key, key))
        })
    }

    pub fn manifest_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(&self.document)
    }

    pub fn encode_acmani(&self) -> Result<Vec<u8>, SchemaError> {
        let manifest = self.manifest_toml()?.into_bytes();
        let default_config = self.default_config_toml().into_bytes();
        let mut hash = Sha256::new();
        hash.update(&manifest);
        hash.update(&default_config);
        let hash = hash.finalize();
        let manifest_len = u32::try_from(manifest.len())
            .map_err(|_| SchemaError::Invalid("manifest 超过 V1 长度限制".to_owned()))?;
        let default_len = u32::try_from(default_config.len())
            .map_err(|_| SchemaError::Invalid("default_config 超过 V1 长度限制".to_owned()))?;
        let total = usize::from(HEADER_LENGTH)
            .checked_add(manifest.len())
            .and_then(|length| length.checked_add(default_config.len()))
            .ok_or_else(|| SchemaError::Invalid("acmani 长度溢出".to_owned()))?;
        let mut blob = vec![0u8; total];
        blob[..8].copy_from_slice(MAGIC);
        blob[8..10].copy_from_slice(&CONTAINER_VERSION.to_le_bytes());
        blob[10..12].copy_from_slice(&HEADER_LENGTH.to_le_bytes());
        blob[12..16].copy_from_slice(&manifest_len.to_le_bytes());
        blob[16..20].copy_from_slice(&default_len.to_le_bytes());
        blob[20..52].copy_from_slice(&hash);
        let header = usize::from(HEADER_LENGTH);
        blob[header..header + manifest.len()].copy_from_slice(&manifest);
        blob[header + manifest.len()..].copy_from_slice(&default_config);
        Ok(blob)
    }
}

pub fn decode_acmani(blob: &[u8]) -> Result<Acmani<'_>, SchemaError> {
    let header = usize::from(HEADER_LENGTH);
    if blob.len() < header {
        return Err(SchemaError::Invalid("acmani 头不完整".to_owned()));
    }
    if &blob[..8] != MAGIC {
        return Err(SchemaError::Invalid("acmani magic 不匹配".to_owned()));
    }
    let version = u16::from_le_bytes([blob[8], blob[9]]);
    if version != CONTAINER_VERSION {
        return Err(SchemaError::Invalid(format!(
            "不支持的 acmani container version: {version}"
        )));
    }
    let header_length = u16::from_le_bytes([blob[10], blob[11]]);
    if header_length != HEADER_LENGTH {
        return Err(SchemaError::Invalid(format!(
            "acmani header length 无效: {header_length}"
        )));
    }
    if blob[52..header].iter().any(|byte| *byte != 0) {
        return Err(SchemaError::Invalid("acmani reserved 字段非零".to_owned()));
    }
    let manifest_len = u32::from_le_bytes([blob[12], blob[13], blob[14], blob[15]]) as usize;
    let default_len = u32::from_le_bytes([blob[16], blob[17], blob[18], blob[19]]) as usize;
    let manifest_end = header
        .checked_add(manifest_len)
        .ok_or_else(|| SchemaError::Invalid("acmani manifest 长度溢出".to_owned()))?;
    let total = manifest_end
        .checked_add(default_len)
        .ok_or_else(|| SchemaError::Invalid("acmani default_config 长度溢出".to_owned()))?;
    if total != blob.len() {
        return Err(SchemaError::Invalid("acmani payload 长度不匹配".to_owned()));
    }
    let manifest_bytes = &blob[header..manifest_end];
    let default_bytes = &blob[manifest_end..total];
    let mut hash = Sha256::new();
    hash.update(manifest_bytes);
    hash.update(default_bytes);
    if hash.finalize()[..] != blob[20..52] {
        return Err(SchemaError::Invalid("acmani SHA-256 不匹配".to_owned()));
    }
    let manifest = std::str::from_utf8(manifest_bytes)
        .map_err(|_| SchemaError::Invalid("acmani manifest 不是 UTF-8".to_owned()))?;
    let default_config = std::str::from_utf8(default_bytes)
        .map_err(|_| SchemaError::Invalid("acmani default_config 不是 UTF-8".to_owned()))?;
    manifest
        .parse::<toml::Value>()
        .map_err(|error| SchemaError::Invalid(format!("acmani manifest TOML 无效: {error}")))?;
    default_config.parse::<toml::Value>().map_err(|error| {
        SchemaError::Invalid(format!("acmani default_config TOML 无效: {error}"))
    })?;
    Ok(Acmani {
        manifest,
        default_config,
    })
}

pub fn decode_pe_acmani(image: &[u8]) -> Result<Acmani<'_>, SchemaError> {
    if read_u16(image, 0)? != 0x5A4D {
        return Err(SchemaError::Invalid("PE DOS magic 不匹配".to_owned()));
    }
    let pe = read_u32(image, 0x3C)? as usize;
    if read_u32(image, pe)? != 0x0000_4550 {
        return Err(SchemaError::Invalid("PE signature 不匹配".to_owned()));
    }
    let section_count = read_u16(image, pe + 6)? as usize;
    let optional_size = read_u16(image, pe + 20)? as usize;
    let section_table = pe
        .checked_add(24)
        .and_then(|offset| offset.checked_add(optional_size))
        .ok_or_else(|| SchemaError::Invalid("PE section table 偏移溢出".to_owned()))?;
    let mut matches = Vec::new();
    for index in 0..section_count {
        let section = section_table
            .checked_add(index * 40)
            .ok_or_else(|| SchemaError::Invalid("PE section 偏移溢出".to_owned()))?;
        let name = image
            .get(section..section + 8)
            .ok_or_else(|| SchemaError::Invalid("PE section table 不完整".to_owned()))?;
        let name_end = name
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(name.len());
        if &name[..name_end] != b".acmani" {
            continue;
        }
        let raw_size = read_u32(image, section + 16)? as usize;
        let raw_offset = read_u32(image, section + 20)? as usize;
        matches.push((raw_offset, raw_size));
    }
    if matches.len() != 1 {
        return Err(SchemaError::Invalid(format!(
            "PE 必须恰好包含一个 .acmani section，实际为 {}",
            matches.len()
        )));
    }
    let (offset, raw_size) = matches[0];
    let raw = image
        .get(offset..offset.saturating_add(raw_size))
        .ok_or_else(|| SchemaError::Invalid(".acmani section 越界".to_owned()))?;
    if raw.len() < usize::from(HEADER_LENGTH) || &raw[..8] != MAGIC {
        return Err(SchemaError::Invalid(
            ".acmani section 缺少 V1 magic".to_owned(),
        ));
    }
    let manifest_len = read_u32(raw, 12)? as usize;
    let default_len = read_u32(raw, 16)? as usize;
    let blob_len = usize::from(HEADER_LENGTH)
        .checked_add(manifest_len)
        .and_then(|length| length.checked_add(default_len))
        .ok_or_else(|| SchemaError::Invalid(".acmani blob 长度溢出".to_owned()))?;
    let blob = raw
        .get(..blob_len)
        .ok_or_else(|| SchemaError::Invalid(".acmani blob 超出 PE section".to_owned()))?;
    decode_acmani(blob)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, SchemaError> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| SchemaError::Invalid("二进制数据截断".to_owned()))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, SchemaError> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| SchemaError::Invalid("二进制数据截断".to_owned()))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn validate_sections(sections: &[SectionSpec]) -> Result<(), SchemaError> {
    for (index, section) in sections.iter().enumerate() {
        if sections[..index]
            .iter()
            .any(|other| keys_equal(&other.id, &section.id))
        {
            return Err(SchemaError::Invalid(format!(
                "重复配置 section: {}",
                section.id
            )));
        }
        let mut keys = Vec::new();
        for entry in &section.entries {
            if !keys.iter().all(|key: &String| !keys_equal(key, &entry.key)) {
                return Err(SchemaError::Invalid(format!(
                    "重复配置项 {}.{}",
                    section.id, entry.key
                )));
            }
            validate_entry(section, entry)?;
            keys.push(entry.key.clone());
        }
        let enable = section
            .entries
            .iter()
            .find(|entry| keys_equal(&entry.key, "enable"));
        if section.hidden || section.always_enabled {
            if enable.is_some() {
                return Err(SchemaError::Invalid(format!(
                    "内置配置栏目 {} 不应公开 enable",
                    section.id
                )));
            }
        } else if !enable.is_some_and(|entry| {
            entry.value_type == "bool"
                && entry.default.as_ref().and_then(toml::Value::as_bool) == Some(section.default_on)
        }) {
            return Err(SchemaError::Invalid(format!(
                "配置栏目 {} 缺少正确的 enable 默认值",
                section.id
            )));
        }
    }
    Ok(())
}

fn enable_entry(default_on: bool) -> EntrySpec {
    EntrySpec {
        key: "enable".to_owned(),
        value_type: "bool".to_owned(),
        default: Some(toml::Value::Boolean(default_on)),
        min: None,
        max: None,
        emit_default: true,
        emit_comment: true,
        hidden: false,
        options: Vec::new(),
        comment: Some(LocalizedText {
            zh: Some("启用".to_owned()),
            en: Some("Enable".to_owned()),
        }),
        description: None,
    }
}

fn inject_enable_entries(
    document: &mut toml::Value,
    sections: &[SectionSpec],
) -> Result<(), SchemaError> {
    let raw_sections = document
        .as_table_mut()
        .and_then(|root| root.get_mut("config"))
        .and_then(toml::Value::as_table_mut)
        .and_then(|config| config.get_mut("sections"))
        .and_then(toml::Value::as_array_mut)
        .ok_or_else(|| SchemaError::Invalid("schema 缺少 config.sections".to_owned()))?;

    for (raw, section) in raw_sections.iter_mut().zip(sections) {
        let Some(enable) = section
            .entries
            .iter()
            .find(|entry| keys_equal(&entry.key, "enable"))
        else {
            continue;
        };
        let table = raw
            .as_table_mut()
            .ok_or_else(|| SchemaError::Invalid(format!("配置栏目 {} 必须是表", section.id)))?;
        let entries = table
            .entry("entries")
            .or_insert_with(|| toml::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                SchemaError::Invalid(format!("配置栏目 {}.entries 必须是数组", section.id))
            })?;
        if entries.iter().any(|entry| {
            entry
                .as_table()
                .and_then(|entry| entry.get("key"))
                .and_then(toml::Value::as_str)
                .is_some_and(|key| keys_equal(key, "enable"))
        }) {
            continue;
        }

        let mut label = toml::Table::new();
        label.insert("zh".to_owned(), toml::Value::String("启用".to_owned()));
        label.insert("en".to_owned(), toml::Value::String("Enable".to_owned()));
        let mut entry = toml::Table::new();
        entry.insert("key".to_owned(), toml::Value::String(enable.key.clone()));
        entry.insert(
            "type".to_owned(),
            toml::Value::String(enable.value_type.clone()),
        );
        entry.insert(
            "default".to_owned(),
            enable.default.clone().expect("enable 必须包含代码默认值"),
        );
        entry.insert("emit_default".to_owned(), toml::Value::Boolean(true));
        entry.insert("label".to_owned(), toml::Value::Table(label));
        entries.insert(0, toml::Value::Table(entry));
    }
    Ok(())
}

fn canonicalize_document_keys(document: &mut toml::Value) -> Result<(), SchemaError> {
    let sections = document
        .as_table_mut()
        .and_then(|root| root.get_mut("config"))
        .and_then(toml::Value::as_table_mut)
        .and_then(|config| config.get_mut("sections"))
        .and_then(toml::Value::as_array_mut)
        .ok_or_else(|| SchemaError::Invalid("schema 缺少 config.sections".to_owned()))?;
    for section in sections {
        let Some(entries) = section
            .as_table_mut()
            .and_then(|section| section.get_mut("entries"))
            .and_then(toml::Value::as_array_mut)
        else {
            continue;
        };
        for entry in entries {
            let Some(key) = entry.as_table_mut().and_then(|entry| entry.get_mut("key")) else {
                continue;
            };
            let raw = key
                .as_str()
                .ok_or_else(|| SchemaError::Invalid("配置项 key 必须是字符串".to_owned()))?;
            *key = toml::Value::String(canonical_key(raw));
        }
    }
    Ok(())
}

fn validate_entry(section: &SectionSpec, entry: &EntrySpec) -> Result<(), SchemaError> {
    let valid_type = matches!(
        entry.value_type.as_str(),
        "bool" | "int" | "float" | "string" | "string_array"
    );
    if !valid_type {
        return Err(SchemaError::Invalid(format!(
            "不支持的配置类型 {}.{}: {}",
            section.id, entry.key, entry.value_type
        )));
    }
    if let (Some(min), Some(max)) = (entry.min, entry.max) {
        if min > max {
            return Err(SchemaError::Invalid(format!(
                "配置范围无效 {}.{}: min > max",
                section.id, entry.key
            )));
        }
    }
    let type_matches = |value: &toml::Value| match entry.value_type.as_str() {
        "bool" => value.is_bool(),
        "int" => value.is_integer(),
        "float" => value.is_float() || value.is_integer(),
        "string" => value.is_str(),
        "string_array" => value
            .as_array()
            .is_some_and(|values| values.iter().all(toml::Value::is_str)),
        _ => false,
    };
    if let Some(default) = &entry.default {
        if !type_matches(default) {
            return Err(SchemaError::Invalid(format!(
                "默认值类型错误 {}.{}",
                section.id, entry.key
            )));
        }
        validate_entry_range(section, entry, default, "默认值")?;
        if !entry.options.is_empty() && !entry.options.iter().any(|option| option.value == *default)
        {
            return Err(SchemaError::Invalid(format!(
                "默认值不在选项中 {}.{}",
                section.id, entry.key
            )));
        }
    }
    for option in &entry.options {
        if !type_matches(&option.value) {
            return Err(SchemaError::Invalid(format!(
                "选项值类型错误 {}.{}",
                section.id, entry.key
            )));
        }
        validate_entry_range(section, entry, &option.value, "选项值")?;
    }
    for (index, option) in entry.options.iter().enumerate() {
        if entry.options[..index]
            .iter()
            .any(|previous| previous.value == option.value)
        {
            return Err(SchemaError::Invalid(format!(
                "重复选项值 {}.{}",
                section.id, entry.key
            )));
        }
    }
    Ok(())
}

fn validate_entry_range(
    section: &SectionSpec,
    entry: &EntrySpec,
    value: &toml::Value,
    label: &str,
) -> Result<(), SchemaError> {
    if let Some(value) = value.as_integer() {
        if entry.min.is_some_and(|min| value < min) || entry.max.is_some_and(|max| value > max) {
            return Err(SchemaError::Invalid(format!(
                "{label}超出范围 {}.{}",
                section.id, entry.key
            )));
        }
    }
    Ok(())
}

fn bool_field(table: &toml::Table, key: &str) -> Result<Option<bool>, SchemaError> {
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| SchemaError::Invalid(format!("配置元数据 {key} 必须是布尔值")))
}

fn parse_entries(table: &toml::Table, section: &str) -> Result<Vec<EntrySpec>, SchemaError> {
    let Some(raw_entries) = table.get("entries") else {
        return Ok(Vec::new());
    };
    let entries = raw_entries
        .as_array()
        .ok_or_else(|| SchemaError::Invalid(format!("配置栏目 {section}.entries 必须是数组")))?;
    entries
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let entry = raw.as_table().ok_or_else(|| {
                SchemaError::Invalid(format!("配置栏目 {section}.entries[{index}] 必须是表"))
            })?;
            let key = entry
                .get("key")
                .and_then(toml::Value::as_str)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    SchemaError::Invalid(format!(
                        "配置栏目 {section}.entries[{index}] 缺少非空 key"
                    ))
                })?;
            let value_type = entry
                .get("type")
                .and_then(toml::Value::as_str)
                .filter(|value_type| !value_type.is_empty())
                .unwrap_or("string");
            Ok(EntrySpec {
                key: key.to_owned(),
                value_type: value_type.to_owned(),
                default: entry.get("default").cloned(),
                min: entry.get("min").and_then(toml::Value::as_integer),
                max: entry.get("max").and_then(toml::Value::as_integer),
                emit_default: entry
                    .get("emit_default")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
                emit_comment: entry
                    .get("emit_comment")
                    .map(|value| {
                        value.as_bool().ok_or_else(|| {
                            SchemaError::Invalid(format!(
                                "配置项 {section}.{key}.emit_comment 必须是布尔值"
                            ))
                        })
                    })
                    .transpose()?
                    .unwrap_or(true),
                hidden: entry
                    .get("hidden")
                    .map(|value| {
                        value.as_bool().ok_or_else(|| {
                            SchemaError::Invalid(format!(
                                "配置项 {section}.{key}.hidden 必须是布尔值"
                            ))
                        })
                    })
                    .transpose()?
                    .unwrap_or(false),
                options: parse_options(entry, section, key)?,
                comment: LocalizedText::from_value(entry.get("label")),
                description: LocalizedText::from_value(entry.get("description")),
            })
        })
        .collect()
}

fn parse_options(
    entry: &toml::Table,
    section: &str,
    key: &str,
) -> Result<Vec<OptionSpec>, SchemaError> {
    let Some(raw_options) = entry.get("options") else {
        return Ok(Vec::new());
    };
    let options = raw_options.as_array().ok_or_else(|| {
        SchemaError::Invalid(format!("配置项 {section}.{key}.options 必须是数组"))
    })?;
    options
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let option = raw.as_table().ok_or_else(|| {
                SchemaError::Invalid(format!("配置项 {section}.{key}.options[{index}] 必须是表"))
            })?;
            let value = option.get("value").cloned().ok_or_else(|| {
                SchemaError::Invalid(format!(
                    "配置项 {section}.{key}.options[{index}] 缺少 value"
                ))
            })?;
            Ok(OptionSpec {
                value,
                label: LocalizedText::from_value(option.get("label")).unwrap_or_default(),
            })
        })
        .collect()
}

fn validate_groups(root: &toml::Table, sections: &[SectionSpec]) -> Result<(), SchemaError> {
    let Some(groups) = root
        .get("ui")
        .and_then(toml::Value::as_table)
        .and_then(|ui| ui.get("groups"))
        .and_then(toml::Value::as_array)
    else {
        return Ok(());
    };
    let mut ids = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let group = group
            .as_table()
            .ok_or_else(|| SchemaError::Invalid(format!("ui.groups[{index}] 必须是表")))?;
        let id = group
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| SchemaError::Invalid(format!("ui.groups[{index}] 缺少 id")))?;
        if ids.iter().any(|other: &&str| keys_equal(other, id)) {
            return Err(SchemaError::Invalid(format!("重复 UI group: {id}")));
        }
        ids.push(id);
        let members = group
            .get("sections")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| SchemaError::Invalid(format!("UI group {id} 缺少 sections")))?;
        for member in members {
            let member = member.as_str().ok_or_else(|| {
                SchemaError::Invalid(format!("UI group {id} 的 section ID 必须是字符串"))
            })?;
            if !sections
                .iter()
                .any(|section| keys_equal(&section.id, member))
            {
                return Err(SchemaError::Invalid(format!(
                    "UI group {id} 引用了未知 section: {member}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        decode_acmani, decode_pe_acmani, generate_from_rust_dir, Schema, CONTAINER_VERSION,
        HEADER_LENGTH,
    };
    use sha2::Digest;

    fn generated_schema() -> Schema {
        generate_from_rust_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../applechu/src"))
            .expect("Rust config declarations must generate schema")
    }

    #[test]
    fn rust_declaration_generates_schema_without_manifest_source() {
        // Given: 临时 Rust 源码只声明配置类型和字段元数据。
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be valid")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "applechu-schema-generator-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("fixture directory must be created");
        fs::write(
            directory.join("fixture.rs"),
            r#"
                crate::config_section! {
                    struct FixtureConfig => FIXTURE_CONFIG_SECTION {
                        section: "Fixture",
                        order: 1,
                        default_on: false,
                        always_enabled: false,
                        hidden: false,
                        group: "display",
                        comment: "Fixture",
                        fields: {
                            version_number: i32 = 3,
                            min: 1,
                            max: 9,
                            comment: "Version text";
                        }
                    }
                }
            "#,
        )
        .expect("fixture source must be written");

        // When: 构建生成器扫描 Rust 声明。
        let schema = generate_from_rust_dir(&directory).expect("fixture schema must generate");
        fs::remove_dir_all(&directory).expect("fixture directory must be removed");

        // Then: manifest 和默认配置都直接包含规范化后的声明内容。
        let entry = schema
            .entry("Fixture", "VersionNumber")
            .expect("generated entry must exist");
        assert_eq!(entry.value_type, "int");
        assert_eq!(
            entry.default.as_ref().and_then(toml::Value::as_integer),
            Some(3)
        );
        assert_eq!(entry.min, Some(1));
        assert_eq!(entry.max, Some(9));
        assert!(schema.default_config_toml().contains("#VersionNumber = 3"));
    }

    #[test]
    fn bundled_schema_round_trips_and_emits_v1() {
        let schema = generated_schema();
        let blob = schema.encode_acmani().expect("acmani must encode");
        assert_eq!(&blob[..8], b"ACMANI\0\0");
        assert_eq!(u16::from_le_bytes([blob[8], blob[9]]), CONTAINER_VERSION);
        assert_eq!(u16::from_le_bytes([blob[10], blob[11]]), HEADER_LENGTH);
        assert!(schema.default_config_toml().parse::<toml::Value>().is_ok());
        let decoded = decode_acmani(&blob).expect("acmani must decode");
        assert_eq!(decoded.manifest, schema.manifest_toml().unwrap());
        assert_eq!(decoded.default_config, schema.default_config_toml());
        assert!(schema
            .section("Unlocker")
            .is_some_and(|section| section.community));

        let emitted = schema
            .manifest_toml()
            .expect("emitted manifest must serialize");
        let reparsed = Schema::parse(emitted).expect("emitted manifest must parse again");
        assert_eq!(
            reparsed
                .sections()
                .iter()
                .map(|section| section.id.as_str())
                .collect::<Vec<_>>(),
            schema
                .sections()
                .iter()
                .map(|section| section.id.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn accepts_default_on_aliases() {
        let source = r#"
            [config]
            [[config.sections]]
            id = "OffByDefault"
            default_on = false
            [[config.sections]]
            id = "OnByDefault"
            defaultOn = true
        "#;
        let schema = Schema::parse(source).expect("aliases must be accepted");
        assert!(!schema.section("OffByDefault").unwrap().default_on);
        assert!(schema.section("OnByDefault").unwrap().default_on);
    }

    #[test]
    fn default_config_matches_section_state_model() {
        let schema = generated_schema();
        let config = schema.default_config_toml();
        let document = config
            .parse::<toml::Table>()
            .expect("default config must be valid TOML");
        let amdaemon = config
            .split("[Amdaemon]\n")
            .nth(1)
            .expect("AM Daemon section must be emitted")
            .split("\n[")
            .next()
            .expect("AM Daemon section body must exist");

        assert!(config.starts_with("## 这是 AppleChu 的 TOML 配置文件"));
        assert!(config.contains("ConfigVersion = 1"));
        assert!(amdaemon.contains("Enable = true"));
        assert!(amdaemon.contains("AutoStart = false"));
        assert!(amdaemon.contains("AppendConfigArgs = false"));
        assert!(amdaemon.contains("#ConfigFiles = [\"config_*.json\"]"));
        assert!(document["DisableEncryption"].as_table().is_some());
        assert!(document["DisableTLS"].as_table().is_some());
        assert!(config.contains("#GameId = \"SDHD\""));
        assert_eq!(
            schema
                .entry("SliderDevice", "enable")
                .and_then(|entry| entry.default.as_ref())
                .and_then(toml::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn system_entries_expose_manager_options() {
        let schema = generated_schema();
        let mode = schema.entry("System", "Mode").expect("Mode must exist");
        assert_eq!(mode.options.len(), 2);
        assert_eq!(mode.options[0].value.as_str(), Some("SP"));
        assert_eq!(mode.options[1].value.as_str(), Some("CVT"));

        assert!(schema.entry("System", "RefreshRate").is_none());
    }

    #[test]
    fn default_config_omits_required_internal_sections() {
        let schema = generated_schema();
        let config = schema.default_config_toml();

        for name in [
            "Clock", "Misc", "AMVideo", "DVD", "Epay", "OpenSsl", "Hwmon", "Hwreset", "HookMode",
        ] {
            assert!(schema.section(name).is_none());
            assert!(!config.contains(&format!("[{name}]")));
        }
        assert!(config.contains("[PCBID]"));
        assert!(config.contains("[VFS]"));
        assert!(config.contains("[SliderDevice]"));
    }

    #[test]
    fn default_config_follows_settings_sort_order() {
        // Given: Rust 声明生成完整的公开配置 schema。
        let schema = generated_schema();

        // When: 默认配置按 section 顺序输出。
        let config = schema.default_config_toml();
        let sections = config
            .lines()
            .filter_map(|line| line.strip_prefix('[')?.strip_suffix(']'))
            .collect::<Vec<_>>();

        // Then: 顺序与设置管理器约定完全一致。
        assert_eq!(
            sections,
            [
                "System",
                "Amdaemon",
                "Dns",
                "Keychip",
                "Aime",
                "Io4",
                "SliderDevice",
                "ChuniIo",
                "Led",
                "Led15093",
                "Vfd",
                "Window",
                "FreePlay",
                "CreditFreeze",
                "SkipStartup",
                "DisableTimer",
                "CustomTimers",
                "UnlockTracks",
                "SkipMapAnimation",
                "Unlocker",
                "ForceSharedAudio",
                "Force2chAudio",
                "CustomVersionText",
                "DisableEncryption",
                "DisableTLS",
                "Unlock120fps",
                "Bypass1080p",
                "Bypass120hz",
                "BypassAppUser",
                "NetLog",
                "Autoplay",
                "FpsDisplay",
                "FrameLock",
                "DpiAware",
                "PCBID",
                "VFS",
                "NetEnv",
            ]
        );
    }

    #[test]
    fn rejects_malformed_entries_instead_of_silently_dropping_them() {
        let source = r#"
            [config]
            [[config.sections]]
            id = "Broken"
            entries = [{ type = "bool" }]
        "#;
        assert!(Schema::parse(source).is_err());
    }

    #[test]
    fn v1_rejects_tampered_header_length_hash_and_utf8() {
        let schema = generated_schema();
        let blob = schema.encode_acmani().expect("acmani must encode");

        let mut bad_magic = blob.clone();
        bad_magic[0] ^= 0xFF;
        assert!(decode_acmani(&bad_magic).is_err());

        let mut bad_length = blob.clone();
        bad_length[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode_acmani(&bad_length).is_err());

        let mut bad_hash = blob.clone();
        bad_hash[20] ^= 0xFF;
        assert!(decode_acmani(&bad_hash).is_err());

        let mut bad_utf8 = blob;
        let manifest_start = usize::from(HEADER_LENGTH);
        bad_utf8[manifest_start] = 0xFF;
        let payload = bad_utf8[manifest_start..].to_vec();
        let digest = sha2::Sha256::digest(&payload);
        bad_utf8[20..52].copy_from_slice(&digest);
        assert!(decode_acmani(&bad_utf8).is_err());
    }

    #[test]
    fn pe_requires_exactly_one_valid_acmani_section() {
        let schema = generated_schema();
        let blob = schema.encode_acmani().expect("acmani must encode");
        let raw_size = (blob.len() + 0x1FF) & !0x1FF;
        let mut pe = vec![0u8; 0x200 + raw_size];
        pe[0..2].copy_from_slice(&0x5A4Du16.to_le_bytes());
        pe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(&0x0000_4550u32.to_le_bytes());
        pe[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        pe[0x94..0x96].copy_from_slice(&0xE0u16.to_le_bytes());
        let section = 0x80 + 24 + 0xE0;
        pe[section..section + 7].copy_from_slice(b".acmani");
        pe[section + 16..section + 20].copy_from_slice(&(raw_size as u32).to_le_bytes());
        pe[section + 20..section + 24].copy_from_slice(&0x200u32.to_le_bytes());
        pe[0x200..0x200 + blob.len()].copy_from_slice(&blob);

        let decoded = decode_pe_acmani(&pe).expect("PE schema must decode");
        assert_eq!(decoded.default_config, schema.default_config_toml());

        pe[0x86..0x88].copy_from_slice(&2u16.to_le_bytes());
        let second = section + 40;
        pe[second..second + 7].copy_from_slice(b".acmani");
        pe[second + 16..second + 20].copy_from_slice(&(raw_size as u32).to_le_bytes());
        pe[second + 20..second + 24].copy_from_slice(&0x200u32.to_le_bytes());
        assert!(decode_pe_acmani(&pe).is_err());
    }
}
