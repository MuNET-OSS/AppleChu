use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::schema::{
    append_comment, find_section, ConfigDiagnostic, ConfigSection, DiagnosticLevel, LoadedSection,
    SectionDescriptor, SectionRef, CONFIG_SECTIONS,
};

const CONFIG_VERSION: &str = "1";
const BANNER: &str = r#"
这是 AppleChu 的 TOML 配置文件

- 井号 # 开头的行为注释，被注释掉的内容不会生效
    - 被注释的配置内容使用一个井号 #，说明文字使用两个井号 ##
- 功能开关统一使用 enable = true/false
- 未填写的配置使用程序默认值
"#;

static GLOBAL_CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug)]
pub enum ConfigError {
    Parse(toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "Failed to parse TOML: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {}

pub struct Config {
    base_dir: PathBuf,
    sections: HashMap<TypeId, LoadedSection>,
    preserved_sections: Vec<(String, toml::Table)>,
    diagnostics: Vec<ConfigDiagnostic>,
    valid: bool,
}

impl Config {
    pub fn global(base_dir: &str) -> &'static Self {
        GLOBAL_CONFIG.get_or_init(|| Self::load(base_dir))
    }

    pub fn parse(base_dir: impl AsRef<Path>, source: &str) -> Result<Self, ConfigError> {
        let root = source.parse::<toml::Table>().map_err(ConfigError::Parse)?;
        Ok(Self::from_table(base_dir.as_ref(), &root))
    }

    pub fn registered_sections() -> Vec<&'static SectionDescriptor> {
        let mut sections = CONFIG_SECTIONS.iter().collect::<Vec<_>>();
        sections.sort_by_key(|section| (section.order, section.name));
        sections
    }

    pub fn section<T: ConfigSection>(&self) -> Option<SectionRef<'_, T>> {
        let loaded = self.sections.get(&TypeId::of::<T>())?;
        let value = loaded.value::<T>()?;
        Some(SectionRef::new(loaded.enabled, value))
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn diagnostics(&self) -> &[ConfigDiagnostic] {
        &self.diagnostics
    }

    pub fn is_valid(&self) -> bool {
        self.valid
    }

    pub fn to_toml(&self) -> String {
        let mut output = String::new();
        append_comment(&mut output, BANNER);
        output.push('\n');
        output.push_str("config_version = 1\n");

        for descriptor in Self::registered_sections() {
            let Some(loaded) = self.sections.get(&(descriptor.type_id)()) else {
                continue;
            };
            if descriptor.builtin() {
                continue;
            }
            output.push('\n');
            append_comment(&mut output, descriptor.comment());
            output.push('[');
            output.push_str(descriptor.name);
            output.push_str("]\n");
            output.push_str("enable = ");
            output.push_str(if loaded.enabled { "true\n" } else { "false\n" });

            (descriptor.serialize_fields)(loaded, &mut output);
        }
        for (name, table) in &self.preserved_sections {
            output.push('\n');
            output.push('[');
            output.push_str(name);
            output.push_str("]\n");
            if let Ok(fields) = toml::to_string(table) {
                output.push_str(&fields);
            }
        }
        output
    }

    pub(super) fn load(base_dir: &str) -> Self {
        let path = Path::new(base_dir).join("AppleChu.toml");
        match fs::read_to_string(&path) {
            Ok(source) => match Self::parse(base_dir, &source) {
                Ok(config) => config,
                Err(error) => Self::invalid(base_dir, error.to_string()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut config = Self::from_table(Path::new(base_dir), &toml::Table::new());
                if let Err(error) = fs::write(&path, "config_version = 1\n") {
                    config.diagnostics.push(ConfigDiagnostic::warning(format!(
                        "Failed to create AppleChu.toml: {error}"
                    )));
                }
                config
            }
            Err(error) => Self::invalid(base_dir, format!("Failed to read AppleChu.toml: {error}")),
        }
    }

    fn invalid(base_dir: &str, message: String) -> Self {
        let mut config = Self::from_table(Path::new(base_dir), &toml::Table::new());
        config.valid = false;
        config.diagnostics.push(ConfigDiagnostic::error(message));
        config
    }

    fn from_table(base_dir: &Path, root: &toml::Table) -> Self {
        let descriptors = Self::registered_sections();
        let mut diagnostics = Vec::new();
        validate_registry(&descriptors, &mut diagnostics);
        validate_document(root, &descriptors, &mut diagnostics);
        warn_unknown_sections(root, &descriptors, &mut diagnostics);

        let mut sections = HashMap::new();
        for descriptor in &descriptors {
            let table = (!descriptor.builtin())
                .then(|| find_section(root, descriptor.name))
                .flatten();
            let loaded = (descriptor.parse)(table, &mut diagnostics);
            sections.insert((descriptor.type_id)(), loaded);
        }

        let preserved_sections = root
            .iter()
            .filter(|(name, value)| {
                let schema = applechu_schema::section(name);
                schema.is_some()
                    && value.is_table()
                    && !descriptors
                        .iter()
                        .any(|descriptor| descriptor.name.eq_ignore_ascii_case(name))
            })
            .filter_map(|(name, value)| value.as_table().map(|table| (name.clone(), table.clone())))
            .collect();

        let valid = !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error);
        Self {
            base_dir: base_dir.to_owned(),
            sections,
            preserved_sections,
            diagnostics,
            valid,
        }
    }
}

fn validate_document(
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

fn validate_registry(
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
    }
}

fn warn_unknown_sections(
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
