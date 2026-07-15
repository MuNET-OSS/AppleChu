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
    pub default_enabled: bool,
    pub always_enabled: bool,
    pub hidden: bool,
    pub comment: &'static str,
    pub type_id: fn() -> TypeId,
    pub parse: fn(Option<&toml::Table>, &mut Vec<ConfigDiagnostic>) -> LoadedSection,
    pub serialize_fields: fn(&LoadedSection, &mut String),
}

#[distributed_slice]
pub static CONFIG_SECTIONS: [SectionDescriptor];

#[derive(Clone)]
pub struct LoadedSection {
    pub descriptor: &'static SectionDescriptor,
    pub enabled: bool,
    pub explicit: bool,
    value: Arc<dyn Any + Send + Sync>,
    explicit_fields: Vec<bool>,
}

impl LoadedSection {
    pub fn new<T: ConfigSection>(
        table: Option<&toml::Table>,
        default_enabled: bool,
        always_enabled: bool,
        value: T,
        explicit_fields: Vec<bool>,
        diagnostics: &mut Vec<ConfigDiagnostic>,
        section: &str,
    ) -> Self {
        let enabled = section_enabled(table, default_enabled, always_enabled, diagnostics, section);
        Self {
            descriptor: T::descriptor(),
            enabled,
            explicit: table.is_some(),
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

fn section_enabled(
    table: Option<&toml::Table>,
    default_enabled: bool,
    always_enabled: bool,
    diagnostics: &mut Vec<ConfigDiagnostic>,
    section: &str,
) -> bool {
    if always_enabled {
        if find_key(table, "Disabled").is_some() {
            diagnostics.push(ConfigDiagnostic::warning(format!(
                "配置栏目 {section} 不支持 Disabled，已忽略"
            )));
        }
        return true;
    }
    let Some(table) = table else {
        return default_enabled;
    };
    let Some(disabled) = find_key(Some(table), "Disabled") else {
        return true;
    };
    let Some(disabled) = bool::parse(disabled) else {
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "配置项 {section}.Disabled 类型错误，已按 false 处理"
        )));
        return true;
    };
    !disabled
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
        if key.eq_ignore_ascii_case("Disabled")
            || known.iter().any(|known| key.eq_ignore_ascii_case(known))
        {
            continue;
        }
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "无法识别配置项 {section}.{key}，规范化时将删除"
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

pub fn append_entry<T: ConfigValue>(output: &mut String, key: &str, value: &T, explicit: bool) {
    if !explicit {
        output.push('#');
    }
    output.push_str(key);
    output.push_str(" = ");
    output.push_str(&value.to_toml().to_string());
    output.push('\n');
}
