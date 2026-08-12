use std::fs;
use std::path::Path;

use crate::{canonical_key, LocalizedText, Schema};

pub const EXAMPLE_CONFIG_FILE: &str = "AppleChu.example.toml";
pub const FULL_CONFIG_FILE: &str = "AppleChu.full.toml";

pub const DEFAULT_CONFIG_HEADER: &str = r#"## 这是 AppleChu 的 TOML 配置文件
##
## - 井号 # 开头的行为注释，被注释掉的内容不会生效
## - 被注释的配置内容使用一个井号 #，说明文字使用两个井号 ##
## - 功能开关统一使用 Enable = true/false
## - 未填写的配置使用程序默认值

ConfigVersion = 1
"#;

impl Schema {
    pub fn default_config_toml(&self) -> String {
        self.config_toml(false)
    }

    pub fn full_config_toml(&self) -> String {
        self.config_toml(true)
    }

    fn config_toml(&self, include_advanced: bool) -> String {
        let mut output = String::from(DEFAULT_CONFIG_HEADER);
        for section in &self.sections {
            output.push('\n');
            append_comment(&mut output, section.label.zh_or_en());
            output.push('[');
            output.push_str(&section.id);
            output.push_str("]\n");
            if let Some(description) = &section.description {
                append_comment(&mut output, description.zh_or_en());
            }
            for entry in &section.entries {
                if entry.advanced && !include_advanced {
                    continue;
                }
                if entry.emit_comment {
                    append_comment(
                        &mut output,
                        entry.comment.as_ref().and_then(LocalizedText::zh_or_en),
                    );
                }
                if !entry.emit_default {
                    output.push('#');
                }
                output.push_str(&canonical_key(&entry.key));
                output.push_str(" = ");
                output.push_str(
                    &entry
                        .default
                        .as_ref()
                        .map_or_else(|| "\"\"".to_owned(), |value| inline_toml(entry, value)),
                );
                output.push('\n');
            }
        }
        output
    }

    pub fn write_example_config(&self, output: impl AsRef<Path>) -> std::io::Result<()> {
        let output = output.as_ref();
        fs::write(output.join(EXAMPLE_CONFIG_FILE), self.default_config_toml())?;
        fs::write(output.join(FULL_CONFIG_FILE), self.full_config_toml())
    }
}

fn append_comment(output: &mut String, comment: Option<&str>) {
    let Some(comment) = comment else { return };
    for line in comment.lines() {
        output.push_str("## ");
        output.push_str(line.trim());
        output.push('\n');
    }
}

fn inline_toml(entry: &crate::EntrySpec, value: &toml::Value) -> String {
    if entry.format.as_deref() == Some("virtual_key") {
        if let Some(code) = value.as_integer() {
            return format!("0x{code:02X}");
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{generate_from_rust_dir, EXAMPLE_CONFIG_FILE, FULL_CONFIG_FILE};

    #[test]
    fn example_config_is_written_to_build_output() {
        // Given: 构建阶段生成的配置 schema 和独立输出目录。
        let schema =
            generate_from_rust_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../applechu/src"))
                .expect("Rust config declarations must generate schema");
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time must be valid")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "applechu-example-config-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("output directory must be created");

        // When: schema 导出构建示例配置。
        schema
            .write_example_config(&directory)
            .expect("example config must be written");

        // Then: 固定文件名中的内容与内嵌默认配置完全一致。
        let example = fs::read_to_string(directory.join(EXAMPLE_CONFIG_FILE))
            .expect("example config must be readable");
        let full = fs::read_to_string(directory.join(FULL_CONFIG_FILE))
            .expect("full config must be readable");
        fs::remove_dir_all(&directory).expect("output directory must be removed");
        assert_eq!(example, schema.default_config_toml());
        assert!(full.parse::<toml::Table>().is_ok());
        assert!(!example.contains("EnableConsole"));
        assert!(full.contains("#EnableConsole = true"));
        assert!(full.contains("#AppendConfigArgs = true"));
    }
}
