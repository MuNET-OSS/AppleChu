use syn::LitStr;

use super::parser::{FieldDecl, SectionDecl};
use super::value::{expression_value, schema_type};
use crate::{canonical_key, SchemaError};

pub(super) fn build(sections: &[SectionDecl]) -> Result<String, SchemaError> {
    let mut document = toml::Table::new();
    document.insert("mod".to_owned(), toml::Value::Table(mod_metadata()));
    document.insert("ui".to_owned(), toml::Value::Table(ui_metadata(sections)));
    let visible = sections
        .iter()
        .filter(|section| should_export(section))
        .map(section_value)
        .collect::<Result<Vec<_>, _>>()?;
    let mut config = toml::Table::new();
    config.insert("sections".to_owned(), toml::Value::Array(visible));
    document.insert("config".to_owned(), toml::Value::Table(config));
    toml::to_string_pretty(&document).map_err(SchemaError::Serialize)
}

fn mod_metadata() -> toml::Table {
    let mut table = toml::Table::new();
    for (key, value) in [
        ("id", "applesaber.applechu"),
        ("name", "AppleChu"),
        ("version", env!("CARGO_PKG_VERSION")),
        ("homepage", "https://github.com/MuNET-OSS/AppleChu"),
        ("license", "Apache-2.0"),
        ("min_loader_version", "1.0.0"),
    ] {
        table.insert(key.to_owned(), toml::Value::String(value.to_owned()));
    }
    table.insert(
        "authors".to_owned(),
        toml::Value::Array(vec![toml::Value::String("Applesaber".to_owned())]),
    );
    table.insert(
        "game_versions".to_owned(),
        toml::Value::Array(vec![toml::Value::String("2.45".to_owned())]),
    );
    table.insert(
        "description".to_owned(),
        localized("CHUNITHM Mod", Some("CHUNITHM Mod")),
    );
    table
}

fn ui_metadata(sections: &[SectionDecl]) -> toml::Table {
    let definitions = [
        ("common", "常用", "Common"),
        ("gameplay", "游戏", "Gameplay"),
        ("display", "显示", "Display"),
        ("audio", "音频", "Audio"),
        ("network", "网络", "Network"),
        ("io", "IO", "IO"),
        ("compatibility", "兼容", "Compatibility"),
    ];
    let groups = definitions
        .into_iter()
        .filter_map(|(id, label, label_en)| {
            let members = sections
                .iter()
                .filter(|section| {
                    should_export(section)
                        && section
                            .group
                            .as_ref()
                            .map_or(id == "common", |group| group.value() == id)
                })
                .map(|section| toml::Value::String(section.name.value()))
                .collect::<Vec<_>>();
            if members.is_empty() {
                return None;
            }
            let mut group = toml::Table::new();
            group.insert("id".to_owned(), toml::Value::String(id.to_owned()));
            group.insert("label".to_owned(), localized(label, Some(label_en)));
            group.insert("sections".to_owned(), toml::Value::Array(members));
            Some(toml::Value::Table(group))
        })
        .collect();
    let mut ui = toml::Table::new();
    ui.insert("groups".to_owned(), toml::Value::Array(groups));
    ui
}

fn section_value(section: &SectionDecl) -> Result<toml::Value, SchemaError> {
    let mut table = toml::Table::new();
    table.insert("id".to_owned(), toml::Value::String(section.name.value()));
    table.insert(
        "default_enabled".to_owned(),
        toml::Value::Boolean(section.default_on.value),
    );
    if section.always_enabled.value {
        table.insert("always_enabled".to_owned(), toml::Value::Boolean(true));
    }
    if section.hidden.value {
        table.insert("hidden".to_owned(), toml::Value::Boolean(true));
    }
    if section.community {
        table.insert("community".to_owned(), toml::Value::Boolean(true));
    }
    table.insert(
        "label".to_owned(),
        localized(&section.comment.value(), Some(&section.name.value())),
    );
    if let Some(description) = &section.description {
        table.insert(
            "description".to_owned(),
            localized(
                &description.value(),
                section
                    .description_en
                    .as_ref()
                    .map(LitStr::value)
                    .as_deref(),
            ),
        );
    }
    table.insert(
        "entries".to_owned(),
        toml::Value::Array(
            section
                .fields
                .iter()
                .map(field_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
    );
    Ok(toml::Value::Table(table))
}

fn field_value(field: &FieldDecl) -> Result<toml::Value, SchemaError> {
    let raw_key = field
        .key
        .as_ref()
        .map_or_else(|| field.name.to_string(), LitStr::value);
    let canonical_key = canonical_key(&raw_key);
    let mut table = toml::Table::new();
    table.insert("key".to_owned(), toml::Value::String(canonical_key.clone()));
    table.insert(
        "type".to_owned(),
        toml::Value::String(
            field
                .schema_type
                .as_ref()
                .map_or_else(|| schema_type(&field.value_type), |value| Ok(value.value()))?,
        ),
    );
    table.insert(
        "default".to_owned(),
        expression_value(field.schema_default.as_ref().unwrap_or(&field.default))?,
    );
    table.insert(
        "emit_default".to_owned(),
        toml::Value::Boolean(field.emit_default),
    );
    if field.advanced {
        table.insert("advanced".to_owned(), toml::Value::Boolean(true));
    }
    table.insert(
        "label".to_owned(),
        localized(&field.comment.value(), Some(&canonical_key)),
    );
    if let Some(description) = &field.description {
        table.insert(
            "description".to_owned(),
            localized(
                &description.value(),
                field.description_en.as_ref().map(LitStr::value).as_deref(),
            ),
        );
    }
    if field.comment.value().is_empty() {
        table.insert("emit_comment".to_owned(), toml::Value::Boolean(false));
    }
    if let Some(min) = &field.min {
        table.insert("min".to_owned(), expression_value(min)?);
    }
    if let Some(max) = &field.max {
        table.insert("max".to_owned(), expression_value(max)?);
    }
    if !field.options.is_empty() {
        table.insert(
            "options".to_owned(),
            toml::Value::Array(
                field
                    .options
                    .iter()
                    .map(expression_value)
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(option_value)
                    .collect(),
            ),
        );
    }
    Ok(toml::Value::Table(table))
}

fn option_value(value: toml::Value) -> toml::Value {
    let label = match &value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) => value.to_string(),
        _ => String::new(),
    };
    let mut option = toml::Table::new();
    option.insert("value".to_owned(), value);
    option.insert("label".to_owned(), localized(&label, Some(&label)));
    toml::Value::Table(option)
}

fn localized(value: &str, en: Option<&str>) -> toml::Value {
    let mut label = toml::Table::new();
    label.insert("zh".to_owned(), toml::Value::String(value.to_owned()));
    label.insert(
        "en".to_owned(),
        toml::Value::String(en.unwrap_or(value).to_owned()),
    );
    toml::Value::Table(label)
}

fn should_export(section: &SectionDecl) -> bool {
    section.export.unwrap_or(!section.hidden.value)
}
