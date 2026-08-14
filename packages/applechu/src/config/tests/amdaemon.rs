use super::Config;

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
    assert!(output.contains("Default = \"127.0.0.1\""));
    assert!(!output.contains("Router"));
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
    assert!(output.contains("Id = \"A69E-01A88888888\""));
    assert!(output.contains("[Dns]"));
    assert!(output.contains("Default = \"127.0.0.1\""));
}

#[test]
fn empty_dns_section_uses_builtin_server_defaults() {
    let config = Config::parse(".", "config_version = 1\n[Dns]\n").expect("TOML 语法必须有效");
    let dns = config
        .section::<crate::amdaemon::DnsConfig>()
        .expect("DNS 配置必须完成注入");
    let output = config.to_toml();

    assert!(dns.enabled);
    assert_eq!(dns.default, "play.mumur.net");
    assert_eq!(dns.aimedb, "aime.mumur.net");
    assert!(output.contains("[Dns]"));
    assert!(output.contains("Default = \"play.mumur.net\""));
    assert!(output.contains("Aimedb = \"aime.mumur.net\""));
    assert!(!output.contains("Title"));
}

#[test]
fn dns_detects_loopback_targets_across_server_fields() {
    // Given: 每个 DNS 服务器字段分别使用一种本机地址写法。
    for entry in [
        "Default = \"localhost\"",
        "Router = \"LOCALHOST.\"",
        "Startup = \"127.8.9.10:8080\"",
        "Billing = \"http://127.0.0.1:8443\"",
        "Aimedb = \"[::1]:22345\"",
        "Title = \"::1\"",
        "Title = \"service.localhost\"",
    ] {
        let source = format!("ConfigVersion = 1\n[Dns]\n{entry}\n");
        let config = Config::parse(".", &source).expect("测试配置必须有效");
        let dns = config
            .section::<crate::amdaemon::DnsConfig>()
            .expect("DNS 配置必须完成注入");

        // When: AM Daemon 判断是否需要本机服务器补丁。
        let required = dns.enabled && dns.requires_localhost_patch();

        // Then: 任一字段指向回环目标都会自动启用补丁。
        assert!(required, "未识别本机 DNS 目标: {entry}");
    }
}

#[test]
fn dns_does_not_enable_localhost_patch_for_remote_or_disabled_mapping() {
    // Given: 一个远程 DNS 映射，以及一个明确禁用的本机映射。
    let remote = Config::parse(
        ".",
        "ConfigVersion = 1\n[Dns]\nDefault = \"server.example.com\"\n",
    )
    .expect("远程 DNS 配置必须有效");
    let disabled = Config::parse(
        ".",
        "ConfigVersion = 1\n[Dns]\nEnable = false\nDefault = \"127.0.0.1\"\n",
    )
    .expect("禁用 DNS 配置必须有效");

    // When: AM Daemon 计算 localhost 补丁状态。
    let remote = remote
        .section::<crate::amdaemon::DnsConfig>()
        .expect("DNS 配置必须完成注入");
    let disabled = disabled
        .section::<crate::amdaemon::DnsConfig>()
        .expect("DNS 配置必须完成注入");

    // Then: 远程目标和禁用的 DNS 均不启用补丁。
    assert!(!remote.requires_localhost_patch());
    assert!(!(disabled.enabled && disabled.requires_localhost_patch()));
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
