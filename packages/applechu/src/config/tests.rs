use super::{Config, DiagnosticLevel};
use crate::amdaemon::{AmdaemonConfig, EpayConfig};
use crate::system_config::SystemConfig;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

crate::config_section! {
    pub struct TestSectionConfig => TEST_SECTION {
        section: "TestSection",
        order: 900,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "测试栏目",
        fields: {
            answer: i32 = 42, comment: "答案";
            label: String = String::from("default"), comment: "标签";
        }
    }
}

#[test]
fn module_section_is_registered_automatically() {
    // Given: 测试栏目只在当前模块通过宏声明。
    // When: 中央配置框架枚举链接期注册表。
    let sections = Config::registered_sections();

    // Then: 无需中央手写列表也能发现测试栏目。
    assert!(sections.iter().any(|section| section.name == "TestSection"));
}

#[test]
fn typed_fields_are_hydrated_from_toml() {
    // Given: 用户显式启用栏目并覆盖两个字段。
    let source =
        "config_version = 1\n[TestSection]\nenable = true\nanswer = 7\nlabel = \"custom\"\n";

    // When: TOML 在边界被解析为模块自己的配置类型。
    let config = Config::parse(".", source).expect("测试配置必须有效");
    let section = config
        .section::<TestSectionConfig>()
        .expect("测试栏目必须完成注入");

    // Then: 模块只读取强类型字段和统一的栏目启用状态。
    assert!(section.enabled);
    assert_eq!(section.answer, 7);
    assert_eq!(section.label, "custom");
}

#[test]
fn canonical_toml_comments_default_values() {
    // Given: 默认关闭的栏目没有出现在用户配置中。
    let config = Config::parse(".", "config_version = 1\n").expect("测试配置必须有效");

    // When: 中央框架从同一份 schema 生成规范 TOML。
    let output = config.to_toml();

    // Then: 栏目和默认字段都作为可直接取消注释的示例输出。
    assert!(output.contains("[TestSection]"));
    assert!(output.contains("enable = false"));
    assert!(output.contains("#answer = 42"));
    assert!(output.contains("#label = \"default\""));
}

#[test]
fn amdaemon_is_a_container_with_independent_controls() {
    let source = "config_version = 1\n[Amdaemon]\n";
    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let section = config
        .section::<AmdaemonConfig>()
        .expect("AM Daemon 配置必须完成注入");
    let output = config.to_toml();

    assert!(section.enabled);
    assert!(!section.auto_start);
    assert!(!section.append_config_args);
    assert_eq!(section.config_files, ["config_*.json"]);
    assert!(output.contains("[Amdaemon]\n"));
    assert!(output.contains("AutoStart = false"));
    assert!(output.contains("AppendConfigArgs = false"));
}

#[test]
fn section_state_uses_code_defaults_and_explicit_overrides() {
    let absent = Config::parse(".", "config_version = 1\n").expect("测试配置必须有效");
    assert!(!absent.section::<TestSectionConfig>().unwrap().enabled);
    assert!(
        absent
            .section::<crate::gfx::WindowConfig>()
            .unwrap()
            .enabled
    );
    assert!(absent.to_toml().contains("[Window]"));

    let present = Config::parse(".", "config_version = 1\n[TestSection]\nenable = true\n")
        .expect("测试配置必须有效");
    assert!(present.section::<TestSectionConfig>().unwrap().enabled);

    let commented = Config::parse(".", "config_version = 1\n").expect("测试配置必须有效");
    assert!(commented.section::<SystemConfig>().unwrap().enabled);
}

#[test]
fn unknown_entries_are_reported_and_removed() {
    // Given: 配置含有一个未知字段和一个未知栏目。
    let source = concat!(
        "Version = \"1\"\n",
        "[TestSection]\n",
        "answer = 7\n",
        "obsolete = true\n",
        "[UnknownSection]\n",
        "value = 1\n",
    );

    // When: 中央框架校验并规范化配置。
    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let output = config.to_toml();

    // Then: 问题可观测，过期内容不会继续传播。
    assert!(config
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.message.contains("TestSection.obsolete")));
    assert!(config
        .diagnostics()
        .iter()
        .any(|diagnostic| diagnostic.message.contains("UnknownSection")));
    assert!(!output.contains("obsolete"));
    assert!(!output.contains("UnknownSection"));
}

#[test]
fn amdaemon_sections_are_normalized_by_game_proxy() {
    let source = concat!(
        "Version = \"1\"\n",
        "[Dns]\n",
        "default = \"127.0.0.1\"\n",
        "[UnknownSection]\n",
        "value = 1\n",
    );

    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let output = config.to_toml();

    assert!(output.contains("[Dns]"));
    assert!(output.contains("default = \"127.0.0.1\""));
    assert!(output.contains("#router = \"\""));
    assert!(!output.contains("[UnknownSection]"));
}

#[test]
fn amdaemon_values_survive_game_side_normalization() {
    let source = concat!(
        "Version = \"1\"\n",
        "[Keychip]\n",
        "id = \"A69E-01A88888888\"\n",
        "[Dns]\n",
        "default = \"127.0.0.1\"\n",
    );

    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let output = config.to_toml();

    assert!(output.contains("[Keychip]"));
    assert!(output.contains("id = \"A69E-01A88888888\""));
    assert!(output.contains("[Dns]"));
    assert!(output.contains("default = \"127.0.0.1\""));
}

#[test]
fn empty_dns_section_is_filled_by_game_proxy() {
    let config = Config::parse(".", "config_version = 1\n[Dns]\n").expect("TOML 语法必须有效");
    let dns = config
        .section::<crate::amdaemon::DnsConfig>()
        .expect("DNS 配置必须完成注入");
    let output = config.to_toml();

    assert!(dns.enabled);
    assert!(dns.default.is_empty());
    assert!(output.contains("[Dns]"));
    assert!(output.contains("#default = \"\""));
    assert!(output.contains("#title = \"\""));
}

#[test]
fn required_internal_sections_are_not_emitted() {
    let source = concat!(
        "Version = \"1\"\n",
        "[Clock]\n",
        "timezone = \"real\"\n",
        "[Hwmon]\n",
        "[PCBID]\n",
        "serialNo = \"ACAE01A99999999\"\n",
        "[VFS]\n",
        "option = \"../option\"\n",
    );
    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let output = config.to_toml();

    for name in [
        "Clock", "Misc", "AMVideo", "DVD", "Epay", "OpenSsl", "Hwmon", "Hwreset", "HookMode",
    ] {
        assert!(!output.contains(&format!("[{name}]")));
    }
    assert!(output.contains("[PCBID]"));
    assert!(output.contains("[VFS]"));
    assert!(output.contains("[SliderDevice]"));
}

#[test]
fn slider_device_is_public_and_enabled_by_default() {
    let config = Config::parse(".", "config_version = 1\n").expect("TOML 语法必须有效");
    let slider = config
        .section::<crate::slider::SliderDeviceConfig>()
        .expect("触摸条设备配置必须完成注入");

    assert!(slider.enabled);
    assert!(config.to_toml().contains("[SliderDevice]"));
    assert!(config.to_toml().contains("enable = true"));

    let disabled = Config::parse(".", "config_version = 1\n[SliderDevice]\nenable = false\n")
        .expect("TOML 语法必须有效");
    assert!(
        !disabled
            .section::<crate::slider::SliderDeviceConfig>()
            .expect("触摸条设备配置必须完成注入")
            .enabled
    );
}

#[test]
fn invalid_version_or_section_shape_rejects_config() {
    // Given: 文件版本不受支持，且已知栏目不是 TOML 表。
    let source = "Version = \"0\"\nWindow = true\n";

    // When: 中央框架完成结构校验。
    let config = Config::parse(".", source).expect("TOML 语法必须有效");

    // Then: 文件不会被当作可应用、可重写的配置。
    assert!(!config.is_valid());
    assert!(
        config
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.level == DiagnosticLevel::Error)
            .count()
            >= 2
    );
}

#[test]
fn module_value_types_validate_their_domain() {
    let source = "Version = \"1\"\n[System]\nMode = \"unknown\"\nRefreshRate = 3\n";
    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let system = config
        .section::<SystemConfig>()
        .expect("系统配置必须完成注入");

    assert!(system.is_sp_mode());
    // 无效模式回退为 SP，SP 对应 DIPSW3 OFF
    assert_eq!(system.dipsw(), [true, true, false]);
    assert_eq!(config.diagnostics().len(), 2);
    assert!(config
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.message.contains("Invalid value")));
}

#[test]
fn refresh_rate_uses_hertz_in_user_config() {
    let source = "Version = \"1\"\n[System]\nRefreshRate = 120\n";
    let config = Config::parse(".", source).expect("测试配置必须有效");
    let system = config
        .section::<SystemConfig>()
        .expect("系统配置必须完成注入");

    assert_eq!(system.dipsw(), [true, false, false]);
    assert!(config.to_toml().contains("RefreshRate = 120"));
}

#[test]
fn optional_user_features_are_disabled_by_default() {
    let disabled = [
        "FreePlay",
        "SkipStartup",
        "DisableTimer",
        "SkipMapAnimation",
        "Unlock120fps",
        "DpiAware",
    ];
    let sections = Config::registered_sections();

    for name in disabled {
        let section = sections
            .iter()
            .find(|section| section.name == name)
            .unwrap_or_else(|| panic!("缺少配置栏目 {name}"));
        assert!(!section.default_on(), "{name} 不应默认启用");
    }
}

#[test]
fn window_defaults_to_fullscreen() {
    let config = Config::parse(".", "config_version = 1\n").expect("测试配置必须有效");
    let window = config
        .section::<crate::gfx::WindowConfig>()
        .expect("窗口配置必须完成注入");

    assert!(window.enabled);
    assert!(!window.windowed);
}

#[test]
fn explicit_enable_overrides_code_defaults() {
    let config = Config::parse(".", "config_version = 1\n[System]\nenable = false\n")
        .expect("测试配置必须有效");

    assert!(!config.section::<SystemConfig>().unwrap().enabled);
}

#[test]
fn built_in_section_ignores_user_values() {
    let config =
        Config::parse(".", "config_version = 1\n[Epay]\nhook = false\n").expect("测试配置必须有效");
    let section = config.section::<EpayConfig>().unwrap();

    assert!(section.enabled);
    assert!(section.hook);
}

#[test]
fn loading_does_not_rewrite_user_config() {
    let directory = temporary_directory("no-writeback");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");
    let source = "config_version = 1\n\n[UnknownSection]\nvalue = 1\n";
    fs::write(&path, source).expect("必须能写入测试配置");

    let _ = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    let output = fs::read_to_string(&path).expect("必须能读回测试配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert_eq!(output, source);
}

#[test]
fn loading_creates_missing_user_config() {
    let directory = temporary_directory("create-missing");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");

    let config = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    let output = fs::read_to_string(&path).expect("必须生成默认配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert!(config.is_valid());
    assert!(output.contains("config_version = 1\n"));
    assert!(output.contains("[Amdaemon]\n"));
    assert!(output.contains("[SliderDevice]\n"));
    assert!(output.contains("[DisableTLS]\n"));
    assert!(
        config
            .section::<crate::patches::network::DisableEncryptionConfig>()
            .unwrap()
            .enabled
    );
    assert!(
        config
            .section::<crate::patches::network::DisableTlsConfig>()
            .unwrap()
            .enabled
    );

    let document = output
        .parse::<toml::Table>()
        .expect("生成的配置必须是有效 TOML");
    assert_eq!(
        document
            .get("config_version")
            .and_then(toml::Value::as_integer),
        Some(1)
    );
    assert!(document.contains_key("Amdaemon"));
    assert!(document.contains_key("SliderDevice"));

    let reparsed = Config::parse(&directory, &output).expect("生成的配置必须能重新解析");
    assert!(reparsed.is_valid());
    assert!(reparsed.diagnostics().iter().all(|diagnostic| {
        !diagnostic
            .message
            .contains("Unknown config section AutoStart")
            && !diagnostic
                .message
                .contains("Unknown config section AppendConfigArgs")
            && !diagnostic
                .message
                .contains("Unknown config section windowed")
            && !diagnostic.message.contains("DisableTLS.aimedb")
            && !diagnostic.message.contains("DisableTLS.default")
    }));
    assert!(
        reparsed
            .section::<crate::patches::network::DisableEncryptionConfig>()
            .unwrap()
            .enabled
    );
    assert!(
        reparsed
            .section::<crate::patches::network::DisableTlsConfig>()
            .unwrap()
            .enabled
    );
}

fn temporary_directory(case: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间必须有效")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "applechu-config-{case}-{}-{nonce}",
        std::process::id()
    ))
}
