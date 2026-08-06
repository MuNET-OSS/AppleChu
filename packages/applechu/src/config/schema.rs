use std::any::{Any, TypeId};
use std::ops::Deref;
use std::sync::Arc;

use linkme::distributed_slice;

use super::value::ConfigValue;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticLevel {
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

impl ConfigDiagnostic {
    pub fn warning(message: String) -> Self {
        Self {
            level: DiagnosticLevel::Warning,
            message,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            message,
        }
    }
}

pub struct SectionDescriptor {
    pub name: &'static str,
    pub order: u16,
    pub default_on: bool,
    pub always_enabled: bool,
    pub hidden: bool,
    pub comment: &'static str,
    pub type_id: fn() -> TypeId,
    pub parse: fn(Option<&toml::Table>, &mut Vec<ConfigDiagnostic>) -> LoadedSection,
    pub serialize_fields: fn(&LoadedSection, &mut String),
    /// 运行时强类型配置实际读取的 TOML 键。用于和统一 schema 做一致性校验
    pub field_keys: &'static [&'static str],
}

impl SectionDescriptor {
    pub fn default_on(&self) -> bool {
        applechu_schema::section(self.name).map_or(self.default_on, |section| section.default_on)
    }

    pub fn always_enabled(&self) -> bool {
        applechu_schema::section(self.name)
            .map_or(self.always_enabled, |section| section.always_enabled)
    }

    pub fn hidden(&self) -> bool {
        applechu_schema::section(self.name).map_or(self.hidden, |section| section.hidden)
    }

    /// schema 中没有声明的运行时栏目属于内置实现，不写入玩家配置
    pub fn builtin(&self) -> bool {
        self.hidden() && applechu_schema::section(self.name).is_none()
    }

    pub fn comment(&self) -> &str {
        applechu_schema::section(self.name)
            .and_then(|section| section.label.zh_or_en())
            .unwrap_or(self.comment)
    }
}

#[distributed_slice]
pub static CONFIG_SECTIONS: [SectionDescriptor];

#[derive(Clone)]
pub struct LoadedSection {
    pub descriptor: &'static SectionDescriptor,
    pub enabled: bool,
    value: Arc<dyn Any + Send + Sync>,
    explicit_fields: Vec<bool>,
}

impl LoadedSection {
    pub fn new<T: ConfigSection>(
        table: Option<&toml::Table>,
        value: T,
        explicit_fields: Vec<bool>,
    ) -> Self {
        let descriptor = T::descriptor();
        let enabled = section_enabled(table, descriptor);
        Self {
            descriptor: T::descriptor(),
            enabled,
            value: Arc::new(value),
            explicit_fields,
        }
    }

    pub fn value<T: ConfigSection>(&self) -> Option<&T> {
        self.value.downcast_ref::<T>()
    }

    pub fn explicit_fields(&self) -> &[bool] {
        &self.explicit_fields
    }
}

pub trait ConfigSection: Default + Clone + Send + Sync + 'static {
    fn descriptor() -> &'static SectionDescriptor;
}

pub struct SectionRef<'a, T> {
    pub enabled: bool,
    value: &'a T,
}

impl<'a, T> SectionRef<'a, T> {
    pub fn new(enabled: bool, value: &'a T) -> Self {
        Self { enabled, value }
    }
}

impl<T> Deref for SectionRef<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}

pub fn find_key<'a>(table: Option<&'a toml::Table>, key: &str) -> Option<&'a toml::Value> {
    table.and_then(|table| {
        table.get(key).or_else(|| {
            table
                .iter()
                .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
                .map(|(_, value)| value)
        })
    })
}

pub fn find_section<'a>(root: &'a toml::Table, name: &str) -> Option<&'a toml::Table> {
    root.get(name)
        .or_else(|| {
            root.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value)
        })
        .and_then(toml::Value::as_table)
}

fn section_enabled(table: Option<&toml::Table>, descriptor: &SectionDescriptor) -> bool {
    if descriptor.always_enabled() {
        return true;
    }
    table.is_some() || (descriptor.builtin() && descriptor.default_on())
}

pub fn warn_unknown_keys(
    table: Option<&toml::Table>,
    section: &str,
    known: &[&str],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    let Some(table) = table else {
        return;
    };
    for key in table.keys() {
        if known.iter().any(|known| key.eq_ignore_ascii_case(known)) {
            continue;
        }
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "Unknown config entry {section}.{key}; it is ignored"
        )));
    }
}

pub fn append_comment(output: &mut String, comment: &str) {
    let comment = comment.trim();
    if comment.is_empty() {
        return;
    }
    for line in comment.lines() {
        output.push_str("## ");
        output.push_str(line.trim());
        output.push('\n');
    }
}

pub fn append_field_comment(output: &mut String, section: &str, key: &str, fallback: &str) {
    let comment = applechu_schema::SCHEMA
        .entry(section, key)
        .and_then(|entry| entry.comment.as_ref())
        .and_then(|comment| comment.zh_or_en())
        .unwrap_or(fallback);
    append_comment(output, comment);
}

pub fn append_entry<T: ConfigValue>(output: &mut String, key: &str, value: &T, explicit: bool) {
    if !explicit {
        output.push('#');
    }
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(&value.to_toml().to_string());
    output.push('\n');
}
