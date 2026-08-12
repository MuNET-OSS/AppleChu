use applechu_schema::generate_from_rust_dir;

#[test]
fn advanced_entry_is_marked_and_omitted_from_default_config() {
    // Given: System.EnableConsole 在 Rust 配置声明中标记为高级项。
    let schema = generate_from_rust_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../applechu/src"))
        .expect("Rust config declarations must generate schema");

    // When: 构建器生成配置工具清单与默认配置。
    let entry = schema
        .entry("System", "EnableConsole")
        .expect("EnableConsole must exist");
    let manifest = schema.manifest_toml().expect("manifest must serialize");
    let default_config = schema.default_config_toml();

    // Then: 清单标记该项，默认配置不提前展开它。
    assert!(entry.advanced);
    assert!(manifest.contains("advanced = true"));
    assert!(!default_config.contains("EnableConsole"));
}

#[test]
fn configured_advanced_entries_match_the_curated_list() {
    // Given: 少改动的配置项按 section 分组列入高级项清单。
    let schema = generate_from_rust_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../applechu/src"))
        .expect("Rust config declarations must generate schema");
    let expected = [
        (
            "Amdaemon",
            &[
                "Executable",
                "TerminateOnExit",
                "AppendConfigArgs",
                "ConfigFiles",
            ][..],
        ),
        (
            "Dns",
            &[
                "Router",
                "Startup",
                "Billing",
                "Aimedb",
                "Title",
                "ReplaceHost",
                "StartupPort",
                "BillingPort",
                "AimedbPort",
            ],
        ),
        (
            "Keychip",
            &[
                "PlatformId",
                "Region",
                "BillingType",
                "SystemFlag",
                "Subnet",
                "BillingCa",
                "BillingPub",
            ],
        ),
        (
            "Aime",
            &[
                "CvtPort",
                "SpPort",
                "HighBaud",
                "FelicaPath",
                "AuthdataPath",
                "AimeGen",
                "FelicaGen",
                "Scan",
                "Gen",
                "ProxyFlag",
            ],
        ),
        (
            "Io4",
            &[
                "Foreground",
                "Ir",
                "Air1",
                "Air2",
                "Air3",
                "Air4",
                "Air5",
                "Air6",
                "Cell1",
                "Cell2",
                "Cell3",
                "Cell4",
                "Cell5",
                "Cell6",
                "Cell7",
                "Cell8",
                "Cell9",
                "Cell10",
                "Cell11",
                "Cell12",
                "Cell13",
                "Cell14",
                "Cell15",
                "Cell16",
                "Cell17",
                "Cell18",
                "Cell19",
                "Cell20",
                "Cell21",
                "Cell22",
                "Cell23",
                "Cell24",
                "Cell25",
                "Cell26",
                "Cell27",
                "Cell28",
                "Cell29",
                "Cell30",
                "Cell31",
                "Cell32",
            ],
        ),
        (
            "Led",
            &[
                "CabLedOutputPipe",
                "CabLedOutputSerial",
                "ControllerLedOutputPipe",
                "ControllerLedOutputSerial",
                "ControllerLedOutputOpeNITHM",
                "SerialPort",
                "SerialBaud",
            ],
        ),
        (
            "Led15093",
            &[
                "Port0",
                "Port1",
                "BoardNumber",
                "ChipNumber",
                "BootChipNumber",
                "FwVer",
                "FwSum",
                "HighBaud",
            ],
        ),
        ("Vfd", &["PortNo", "UtfConversion"]),
        ("VFS", &["AllowAmfsDownloads"]),
        (
            "NetEnv",
            &["AddrSuffix", "RouterSuffix", "MacAddr", "Broadcast"],
        ),
    ];

    // When/Then: 每个清单项都由 schema 标记为高级项。
    for (section, keys) in expected {
        for key in keys {
            assert!(
                schema
                    .entry(section, key)
                    .is_some_and(|entry| entry.advanced),
                "{section}.{key} must be advanced"
            );
        }
    }
}
