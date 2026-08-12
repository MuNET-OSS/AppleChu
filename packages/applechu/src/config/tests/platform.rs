use super::Config;

#[test]
fn platform_hook_sections_are_public_and_enabled_by_default() {
    // Given: 用户没有覆盖平台 hook 的默认状态。
    let config = Config::parse(".", "ConfigVersion = 1\n").expect("测试配置必须有效");

    // When: 配置被规范化保存。
    let output = config.to_toml();

    // Then: VFS 与 PCBID 都公开默认启用开关。
    assert!(output.contains("[PCBID]\nEnable = true\n"));
    assert!(output.contains("[VFS]\nEnable = true\n"));
}

#[test]
fn platform_hook_modules_follow_explicit_enable_state() {
    // Given: 用户明确关闭 VFS 与 PCBID hook。
    let config = Config::parse(
        ".",
        "ConfigVersion = 1\n[PCBID]\nEnable = false\n[VFS]\nEnable = false\n",
    )
    .expect("测试配置必须有效");

    // When: 模块注册表计算两个 init 的启用状态。
    let modules = crate::module_registry::registered_modules();
    let pcbid = modules
        .iter()
        .find(|module| module.name.ends_with("platform::pcbid::init"))
        .expect("PCBID 模块必须完成注册");
    let vfs = modules
        .iter()
        .find(|module| module.name.ends_with("platform::vfs::init"))
        .expect("VFS 模块必须完成注册");

    // Then: 两个 hook 初始化入口都被配置门控阻止。
    assert!(!(pcbid.enabled)(&config));
    assert!(!(vfs.enabled)(&config));
}
