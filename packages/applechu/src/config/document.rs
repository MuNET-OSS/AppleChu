use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::schema::{
    append_comment, find_section, ConfigDiagnostic, ConfigSection, DiagnosticLevel, LoadedSection,
    SectionDescriptor, SectionRef, CONFIG_SECTIONS,
};
use super::validation::{validate_document, validate_registry, warn_unknown_sections};

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

    pub(crate) fn sync(&self) -> std::io::Result<()> {
        if !self.valid {
            return Ok(());
        }
        fs::write(self.base_dir.join("AppleChu.toml"), self.to_toml())
    }

    pub fn to_toml(&self) -> String {
        let mut output = String::from(applechu_schema::DEFAULT_CONFIG_HEADER);

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
            if !descriptor.hidden() && !descriptor.always_enabled() {
                output.push_str("Enable = ");
                output.push_str(if loaded.enabled { "true\n" } else { "false\n" });
            }

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
                let defaults = crate::schema_embed::SCHEMA.default_config_toml();
                match Self::parse(base_dir, &defaults) {
                    Ok(config) => config,
                    Err(error) => Self::invalid(base_dir, error.to_string()),
                }
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
                let schema = crate::schema_embed::section(name);
                schema.is_some()
                    && value.is_table()
                    && !descriptors
                        .iter()
                        .any(|descriptor| applechu_schema::keys_equal(descriptor.name, name))
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
