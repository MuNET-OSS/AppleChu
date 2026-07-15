use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::Config;
use crate::util::api::Api;

use super::{registered_modules, InitStage};

static CALLED: AtomicBool = AtomicBool::new(false);

crate::config_section! {
    struct RegistryTestConfig => REGISTRY_TEST_CONFIG_SECTION {
        section: "RegistryTest",
        order: 999,
        default_enabled: false,
        always_enabled: false,
        hidden: true,
        comment: "模块注册测试",
        fields: {}
    }
}

#[applechu_macros::config_section(stage = Late, order = 999)]
fn init(_api: &Api, _config: &RegistryTestConfig) {
    CALLED.store(true, Ordering::Relaxed);
}

#[test]
fn config_section_attribute_registers_and_gates_module() {
    // Given: init 只通过属性声明，配置默认关闭。
    let disabled = Config::parse(".", "Version = \"1\"\n").expect("测试配置必须有效");
    let enabled =
        Config::parse(".", "Version = \"1\"\n[RegistryTest]\n").expect("测试配置必须有效");

    // When: 中央模块注册表枚举链接期声明。
    let module = registered_modules()
        .into_iter()
        .find(|module| module.name.ends_with("module_registry::tests::init"))
        .expect("属性声明必须进入注册表");

    // Then: 阶段、顺序和配置门控均由生成代码提供。
    assert_eq!(module.stage, InitStage::Late);
    assert_eq!(module.order, 999);
    assert!(!(module.enabled)(&disabled));
    assert!((module.enabled)(&enabled));
    assert!(!CALLED.load(Ordering::Relaxed));
}

#[test]
fn built_in_modules_are_discovered_in_stage_order() {
    let modules = registered_modules();
    let expected = [
        ("gfx::windowed::init", InitStage::Graphics, 10),
        ("gfx::monitor::init", InitStage::Graphics, 20),
        ("gfx::d3d9::init", InitStage::Graphics, 30),
        ("iohook::proc_addr::init", InitStage::PlatformCore, 10),
        ("platform::reg_hook::init", InitStage::Platform, 10),
        ("platform::vfs::init", InitStage::Platform, 20),
        ("platform::amvideo::init", InitStage::Platform, 30),
        ("platform::clock::init", InitStage::Platform, 40),
        ("platform::misc::init", InitStage::Platform, 50),
        ("platform::pcbid::init", InitStage::Platform, 60),
        ("platform::dvd::init", InitStage::Platform, 70),
        ("platform::system::init", InitStage::Platform, 80),
        ("iohook::init_all", InitStage::IoHook, 10),
        ("chuniio::init", InitStage::Device, 10),
        ("io4::init", InitStage::Device, 20),
        ("slider::init", InitStage::Device, 30),
        ("vfd::init", InitStage::Device, 40),
        ("led::init", InitStage::Device, 50),
        ("aime::init", InitStage::Device, 60),
        ("national_match::init", InitStage::Late, 10),
        ("autoplay::init_all", InitStage::Late, 20),
        ("ux::dpi_aware::init", InitStage::Late, 30),
        ("d3d9::init_all", InitStage::Late, 40),
    ];

    for (name, stage, order) in expected {
        let module = modules
            .iter()
            .find(|module| module.name.ends_with(name))
            .unwrap_or_else(|| panic!("模块未自动注册: {name}"));
        assert_eq!(module.stage, stage, "模块阶段错误: {name}");
        assert_eq!(module.order, order, "模块顺序错误: {name}");
    }

    assert!(modules.windows(2).all(|modules| {
        let left = modules[0];
        let right = modules[1];
        (left.stage, left.order, left.name) <= (right.stage, right.order, right.name)
    }));
}
