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
- 将默认关闭的栏目取消注释即可启用
- 若要禁用默认开启的栏目，请在栏目下添加 Disabled = true
- 配置文件会在启动时按当前程序中的声明重新生成
"#;

static GLOBAL_CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug)]
pub enum ConfigError {
    Parse(toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "TOML 解析失败: {error}"),
        }
    }
}

impl std::error::Error for ConfigError {}

pub struct Config {
    base_dir: PathBuf,
    sections: HashMap<TypeId, LoadedSection>,
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

    pub fn sync(&self) -> std::io::Result<()> {
        if !self.valid {
            return Ok(());
        }
        fs::write(self.base_dir.join("AppleChu.toml"), self.to_toml())
    }

    pub fn to_toml(&self) -> String {
        let mut output = String::new();
        append_comment(&mut output, BANNER);
        output.push('\n');
        output.push_str("Version = \"");
        output.push_str(CONFIG_VERSION);
        output.push_str("\"\n");

        for descriptor in Self::registered_sections() {
            let Some(loaded) = self.sections.get(&(descriptor.type_id)()) else {
                continue;
            };
            if descriptor.hidden && !loaded.explicit {
                continue;
            }
            output.push('\n');
            append_comment(&mut output, descriptor.comment);
            if !loaded.explicit && !descriptor.default_enabled {
                output.push_str("#[");
            } else {
                output.push('[');
            }
            output.push_str(descriptor.name);
            output.push_str("]\n");

            if !descriptor.always_enabled {
                if loaded.explicit && !loaded.enabled {
                    output.push_str("Disabled = true\n");
                } else if descriptor.default_enabled {
                    output.push_str("#Disabled = false\n");
                }
            }
            (descriptor.serialize_fields)(loaded, &mut output);
        }
        output
    }

    fn load(base_dir: &str) -> Self {
        let path = Path::new(base_dir).join("AppleChu.toml");
        match fs::read_to_string(&path) {
            Ok(source) => match Self::parse(base_dir, &source) {
                Ok(config) => config,
                Err(error) => Self::invalid(base_dir, error.to_string()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Self::from_table(Path::new(base_dir), &toml::Table::new())
            }
            Err(error) => Self::invalid(base_dir, format!("读取 AppleChu.toml 失败: {error}")),
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
        for descriptor in descriptors {
            let loaded = (descriptor.parse)(find_section(root, descriptor.name), &mut diagnostics);
            sections.insert((descriptor.type_id)(), loaded);
        }

        let valid = !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.level == DiagnosticLevel::Error);
        Self {
            base_dir: base_dir.to_owned(),
            sections,
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
                "配置栏目大小写重复: {key}"
            )));
            continue;
        }

        if key.eq_ignore_ascii_case("Version") {
            if value.as_str() != Some(CONFIG_VERSION) {
                diagnostics.push(ConfigDiagnostic::error(format!(
                    "不支持的配置版本，当前版本必须为 {CONFIG_VERSION}"
                )));
            }
            continue;
        }

        if descriptors
            .iter()
            .any(|descriptor| key.eq_ignore_ascii_case(descriptor.name))
            && !value.is_table()
        {
            diagnostics.push(ConfigDiagnostic::error(format!(
                "配置栏目 {key} 必须是 TOML 表"
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
                "配置栏目重复注册: {}",
                descriptor.name
            )));
        }
        if !types.insert((descriptor.type_id)()) {
            diagnostics.push(ConfigDiagnostic::error(format!(
                "配置类型重复注册: {}",
                descriptor.name
            )));
        }
    }
}

fn warn_unknown_sections(
    root: &toml::Table,
    descriptors: &[&'static SectionDescriptor],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    for key in root.keys() {
        if key.eq_ignore_ascii_case("Version")
            || descriptors
                .iter()
                .any(|descriptor| key.eq_ignore_ascii_case(descriptor.name))
        {
            continue;
        }
        diagnostics.push(ConfigDiagnostic::warning(format!(
            "无法识别配置栏目 {key}，规范化时将删除"
        )));
    }
}
