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
