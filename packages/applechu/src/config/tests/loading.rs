use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use super::Config;

#[test]
fn loading_does_not_rewrite_existing_user_config() {
    let directory = temporary_directory("read-existing");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");
    let source = concat!(
        "config_version = 1\n",
        "[TestSection]\n",
        "enable = true\n",
        "answer = 7\n",
        "[UnknownSection]\n",
        "value = 1\n",
    );
    fs::write(&path, source).expect("必须能写入测试配置");

    let _ = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    let output = fs::read_to_string(&path).expect("必须能读回测试配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert_eq!(output, source);
}

#[test]
fn loading_preserves_invalid_user_config() {
    let directory = temporary_directory("preserve-invalid");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");
    let source = "config_version = 0\n";
    fs::write(&path, source).expect("必须能写入测试配置");

    let config = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    let output = fs::read_to_string(&path).expect("必须能读回测试配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert!(!config.is_valid());
    assert_eq!(output, source);
}

#[test]
fn loading_does_not_create_missing_user_config() {
    let directory = temporary_directory("read-missing");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");

    let config = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    let output = fs::read_to_string(&path).ok();
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert!(config.is_valid());
    assert!(output.is_none());
}

#[test]
fn sync_writes_canonical_config_for_game_startup() {
    let directory = temporary_directory("sync");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");
    let source = "config_version = 1\n[TestSection]\nenable = true\nanswer = 7\n";
    fs::write(&path, source).expect("必须能写入测试配置");

    let config = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    config.sync().expect("游戏侧启动必须能写入规范配置");
    let output = fs::read_to_string(&path).expect("必须能读回规范配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert!(output.starts_with(applechu_schema::DEFAULT_CONFIG_HEADER));
    assert!(output.contains("[TestSection]\nenable = true"));
    assert!(output.contains("answer = 7"));
}

#[test]
fn sync_does_not_rewrite_invalid_config() {
    let directory = temporary_directory("invalid-sync");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");
    let source = "config_version = 0\n";
    fs::write(&path, source).expect("必须能写入测试配置");

    let config = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    config.sync().expect("无效配置不应报告写入错误");
    let output = fs::read_to_string(&path).expect("必须能读回原始配置");
    fs::remove_dir_all(&directory).expect("必须清理测试目录");

    assert_eq!(output, source);
}

#[test]
fn missing_config_can_be_normalized_by_game_startup() {
    let directory = temporary_directory("create-missing");
    fs::create_dir_all(&directory).expect("必须能创建测试目录");
    let path = directory.join("AppleChu.toml");

    let config = Config::load(directory.to_str().expect("测试路径必须是 UTF-8"));
    config.sync().expect("游戏侧启动必须能生成默认配置");
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
