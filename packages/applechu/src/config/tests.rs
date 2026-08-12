use super::{Config, DiagnosticLevel};
use crate::amdaemon::{AmdaemonConfig, CreditFreezeConfig, EpayConfig};
use crate::system_config::SystemConfig;

mod amdaemon;
mod loading;
mod platform;

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
    assert!(output.contains("Enable = false"));
    assert!(output.contains("#Answer = 42"));
    assert!(output.contains("#Label = \"default\""));
}

#[test]
fn canonical_toml_uses_pascal_case_keys() {
    // Given: 旧配置混用 snake_case、camelCase 和 PascalCase。
    let source = concat!(
        "config_version = 1\n",
        "[General]\n",
        "enable = true\n",
        "versionText = \"2.50\"\n",
    );
    let config = Config::parse(".", source).expect("旧配置必须保持兼容");

    // When: 配置被规范化保存。
    let output = config.to_toml();

    // Then: 根键、开关和 Rust snake_case 字段统一保存为 PascalCase。
    assert!(output.contains("ConfigVersion = 1"));
    assert!(output.contains("[CustomVersionText]\nEnable = true"));
    assert!(!output.contains("[General]"));
    assert!(output.contains("VersionText = \"2.50\""));
    assert!(!output.contains("config_version ="));
    assert!(!output.contains("versionText ="));
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
fn credit_freeze_has_independent_section() {
    let source = concat!(
        "config_version = 1\n",
        "[CreditFreeze]\n",
        "enable = false\n",
    );
    let config = Config::parse(".", source).expect("测试配置必须有效");

    assert!(
        !config
            .section::<CreditFreezeConfig>()
            .expect("credit section 必须存在")
            .enabled
    );
    assert!(
        config
            .section::<AmdaemonConfig>()
            .expect("AM Daemon section 必须存在")
            .enabled
    );
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
fn empty_section_keeps_explicit_enabled_state_when_serialized() {
    let source = "config_version = 1\n[BypassAppUser]\nenable = true\n";
    let config = Config::parse(".", source).expect("测试配置必须有效");
    let output = config.to_toml();
    let reparsed = Config::parse(".", &output).expect("序列化配置必须有效");

    assert!(output.contains("[BypassAppUser]\nEnable = true"));
    assert!(reparsed
        .to_toml()
        .contains("[BypassAppUser]\nEnable = true"));
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
fn slider_device_is_public_and_enabled_by_default() {
    let config = Config::parse(".", "config_version = 1\n").expect("TOML 语法必须有效");
    let slider = config
        .section::<crate::slider::SliderDeviceConfig>()
        .expect("触摸条设备配置必须完成注入");

    assert!(slider.enabled);
    assert!(config.to_toml().contains("[SliderDevice]"));
    assert!(config.to_toml().contains("Enable = true"));

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
fn io4_owns_keyboard_input_mapping() {
    // Given: IO4 栏目覆盖按钮、红外和触摸条映射。
    let source = concat!(
        "config_version = 1\n",
        "[Io4]\n",
        "test = 65\n",
        "air1 = 66\n",
        "cell1 = 67\n",
        "[Aime]\n",
        "iodll = \"aimeio.dll\"\n",
    );

    // When: 配置在边界完成解析。
    let config = Config::parse(".", source).expect("测试配置必须有效");
    let io4 = config
        .section::<crate::io4::Io4Config>()
        .expect("IO4 配置必须完成注入");
    let chuniio = crate::chuniio::config::ChuniIoConfig::load(&config);
    let aime = config
        .section::<crate::aime::AimeSectionConfig>()
        .expect("Aime 配置必须完成注入");

    // Then: IO4 类型直接持有所有输入映射。
    assert_eq!(io4.test, 65);
    assert_eq!(io4.air1, 66);
    assert_eq!(io4.cell1, 67);
    assert_eq!(chuniio.vk_test, 65);
    assert_eq!(chuniio.vk_ir[0], 66);
    assert_eq!(chuniio.vk_cell[0], 67);
    assert_eq!(aime.iodll, "aimeio.dll");
}

#[test]
fn canonical_io_config_has_no_legacy_sections_or_repeated_comments() {
    // Given: 使用全部默认值的配置。
    let config = Config::parse(".", "config_version = 1\n").expect("测试配置必须有效");

    // When: 生成规范 TOML。
    let output = config.to_toml();

    // Then: IO 字段只属于 Io4/Aime，连续映射只说明第一个字段。
    assert!(!output.contains("[Buttons]"));
    assert!(!output.contains("[Air]"));
    assert!(!output.contains("[Slider]"));
    assert!(!output.contains("[AimeIo]"));
    assert!(output.contains("#Iodll = \"\""));
    assert!(output.contains("## 第 1 组红外传感器按键\n#Air1"));
    assert!(!output.contains("## 第 2 组红外传感器按键"));
    assert!(output.contains("## 触摸条第 1 单元按键\n#Cell1"));
    assert!(!output.contains("## 触摸条第 2 单元按键"));
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
