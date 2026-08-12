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
