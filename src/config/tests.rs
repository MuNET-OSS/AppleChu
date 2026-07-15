use super::{Config, DiagnosticLevel};
use crate::gfx::d3d9::D3D9ExConfig;
use crate::system_config::SystemConfig;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

crate::config_section! {
    pub struct TestSectionConfig => TEST_SECTION {
        section: "TestSection",
        order: 900,
        default_enabled: false,
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
    let source = "Version = \"1\"\n[TestSection]\nanswer = 7\nlabel = \"custom\"\n";

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
    let config = Config::parse(".", "Version = \"1\"\n").expect("测试配置必须有效");

    // When: 中央框架从同一份 schema 生成规范 TOML。
    let output = config.to_toml();

    // Then: 栏目和默认字段都作为可直接取消注释的示例输出。
    assert!(output.contains("#[TestSection]"));
    assert!(output.contains("#answer = 42"));
    assert!(output.contains("#label = \"default\""));
}

#[test]
fn device_lost_fix_is_not_a_registered_section() {
    // Given: D3D9Ex 是设备丢失恢复功能的唯一配置所有者。
    // When: 枚举完整配置 schema。
    let sections = Config::registered_sections();

    // Then: 旧 DeviceLostFix 栏目不会继续泄漏到用户配置。
    assert!(!sections
        .iter()
        .any(|section| section.name.eq_ignore_ascii_case("DeviceLostFix")));
}

#[test]
fn disabled_is_applied_centrally() {
    // Given: 用户通过统一语义关闭一个默认开启的栏目。
    let source = "Version = \"1\"\n[D3D9Ex]\nDisabled = true\n";

    // When: 中央框架注入模块配置。
    let config = Config::parse(".", source).expect("测试配置必须有效");
    let section = config
        .section::<D3D9ExConfig>()
        .expect("D3D9Ex 必须完成注入");

    // Then: 模块获得关闭状态，规范化结果保留用户选择。
    assert!(!section.enabled);
    assert!(config.to_toml().contains("[D3D9Ex]\nDisabled = true"));
}

#[test]
fn d3d9ex_owns_device_lost_recovery() {
    // Given: 设备丢失恢复只配置在 D3D9Ex 中。
    let source = concat!(
        "Version = \"1\"\n",
        "[D3D9Ex]\n",
        "device_lost_recover = false\n",
        "fast_restart = false\n",
    );

    // When: 配置被注入并重新序列化。
    let config = Config::parse(".", source).expect("测试配置必须有效");
    let section = config
        .section::<D3D9ExConfig>()
        .expect("D3D9Ex 必须完成注入");
    let output = config.to_toml();

    // Then: 两个 D3D9 行为来自同一类型，输出不再生成旧栏目。
    assert!(!section.device_lost_recover);
    assert!(!section.fast_restart);
    assert!(output.contains("device_lost_recover = false"));
    assert!(!output.contains("[DeviceLostFix]"));
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
fn invalid_version_or_section_shape_rejects_config() {
    // Given: 文件版本不受支持，且已知栏目不是 TOML 表。
    let source = "Version = \"0\"\nD3D9Ex = true\n";

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
fn sync_writes_canonical_config() {
    let directory = temporary_directory("sync");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let config = Config::parse(&directory, "Version = \"1\"\n[TestSection]\nanswer = 7\n")
        .expect("测试配置必须有效");

    config.sync().expect("规范配置必须能写入");
    let output = fs::read_to_string(directory.join("AppleChu.toml")).expect("必须能读回规范配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert!(output.contains("Version = \"1\""));
    assert!(output.contains("[TestSection]"));
    assert!(output.contains("answer = 7"));
    assert!(!output.contains("DeviceLostFix"));
}

#[test]
fn invalid_config_is_not_rewritten() {
    let directory = temporary_directory("invalid");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");
    let source = "Version = \"0\"\n";
    fs::write(&path, source).expect("必须能写入测试配置");
    let config = Config::parse(&directory, source).expect("TOML 语法必须有效");

    config.sync().expect("拒绝重写不是 IO 错误");
    let output = fs::read_to_string(&path).expect("必须能读回原始配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert_eq!(output, source);
}

#[test]
fn module_value_types_validate_their_domain() {
    let source = "Version = \"1\"\n[System]\nMode = \"unknown\"\nRefreshRate = 3\n";
    let config = Config::parse(".", source).expect("TOML 语法必须有效");
    let system = config
        .section::<SystemConfig>()
        .expect("系统配置必须完成注入");

    assert!(system.is_sp_mode());
    assert_eq!(system.dipsw(), [true, true, true]);
    assert_eq!(config.diagnostics().len(), 2);
    assert!(config
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.message.contains("值无效")));
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
        "D3D9Ex",
        "FreePlay",
        "SkipStartup",
        "DisableTimer",
        "SkipMapAnimation",
        "Unlock120fps",
        "DisableEncryption",
        "DisableTLS",
        "DpiAware",
    ];
    let sections = Config::registered_sections();

    for name in disabled {
        let section = sections
            .iter()
            .find(|section| section.name == name)
            .unwrap_or_else(|| panic!("缺少配置栏目 {name}"));
        assert!(!section.default_enabled, "{name} 不应默认启用");
    }
}

#[test]
fn window_defaults_to_fullscreen() {
    let config = Config::parse(".", "Version = \"1\"\n").expect("测试配置必须有效");
    let window = config
        .section::<crate::gfx::WindowConfig>()
        .expect("窗口配置必须完成注入");

    assert!(window.enabled);
    assert!(!window.windowed);
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
