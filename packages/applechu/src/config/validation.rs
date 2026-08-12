use std::collections::HashSet;

use super::schema::{ConfigDiagnostic, SectionDescriptor};

const CONFIG_VERSION: &str = "1";

pub(super) fn validate_document(
    root: &toml::Table,
    descriptors: &[&'static SectionDescriptor],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    let mut root_keys = HashSet::new();
    for (key, value) in root {
        let normalized = key.to_ascii_lowercase();
        if !root_keys.insert(normalized) {
            diagnostics.push(ConfigDiagnostic::error(format!(
                "Duplicate config section with different casing: {key}"
            )));
            continue;
        }

        if key.eq_ignore_ascii_case("config_version") {
            if value.as_integer() != Some(1) {
                diagnostics.push(ConfigDiagnostic::error(format!(
                    "Unsupported config version; expected {CONFIG_VERSION}"
                )));
            }
            continue;
        }
        if key.eq_ignore_ascii_case("Version") {
            if value.as_str() != Some(CONFIG_VERSION) {
                diagnostics.push(ConfigDiagnostic::error(format!(
                    "Unsupported config version; expected {CONFIG_VERSION}"
                )));
            }
            continue;
        }

        if (descriptors
            .iter()
            .any(|descriptor| !descriptor.builtin() && key.eq_ignore_ascii_case(descriptor.name))
            || applechu_schema::section(key).is_some())
            && !value.is_table()
        {
            diagnostics.push(ConfigDiagnostic::error(format!(
                "Config section {key} must be a TOML table"
            )));
        }
    }
}

pub(super) fn validate_registry(
    descriptors: &[&'static SectionDescriptor],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    let mut names = HashSet::new();
    let mut types = HashSet::new();
    for descriptor in descriptors {
        if !names.insert(descriptor.name.to_ascii_lowercase()) {
            diagnostics.push(ConfigDiagnostic::error(format!(
                "Duplicate config section registration: {}",
                descriptor.name
            )));
        }
        if !types.insert((descriptor.type_id)()) {
            diagnostics.push(ConfigDiagnostic::error(format!(
                "Duplicate config type registration: {}",
                descriptor.name
            )));
        }
        if let Some(schema) = applechu_schema::section(descriptor.name) {
            validate_schema_metadata(descriptor, schema, diagnostics);
        }
    }
}

fn validate_schema_metadata(
    descriptor: &SectionDescriptor,
    schema: &applechu_schema::SectionSpec,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    if schema.default_on != descriptor.default_on
        || schema.always_enabled != descriptor.always_enabled
        || schema.hidden != descriptor.hidden
    {
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "Runtime fallback metadata for section {} differs from the schema; using the schema",
            descriptor.name
        )));
    }

    let schema_keys = schema
        .entries
        .iter()
        .filter(|entry| !entry.key.eq_ignore_ascii_case("enable"))
        .map(|entry| entry.key.as_str())
        .collect::<Vec<_>>();
    let missing = descriptor
        .field_keys
        .iter()
        .copied()
        .filter(|key| {
            !schema_keys
                .iter()
                .any(|schema_key| schema_key.eq_ignore_ascii_case(key))
        })
        .collect::<Vec<_>>();
    let extra = schema_keys
        .iter()
        .copied()
        .filter(|key| {
            !descriptor
                .field_keys
                .iter()
                .any(|runtime_key| runtime_key.eq_ignore_ascii_case(key))
        })
        .collect::<Vec<_>>();
    if !missing.is_empty() || !extra.is_empty() {
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "Runtime fields for section {} differ from the schema; runtime missing [{}], schema extra [{}]",
            descriptor.name,
            missing.join(", "),
            extra.join(", "),
        )));
    }
}

pub(super) fn warn_unknown_sections(
    root: &toml::Table,
    descriptors: &[&'static SectionDescriptor],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    for key in root.keys() {
        if key.eq_ignore_ascii_case("config_version")
            || key.eq_ignore_ascii_case("Version")
            || descriptors.iter().any(|descriptor| {
                !descriptor.builtin() && key.eq_ignore_ascii_case(descriptor.name)
            })
            || applechu_schema::section(key).is_some()
        {
            continue;
        }
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "Unknown config section {key}; it is ignored"
        )));
    }
}
