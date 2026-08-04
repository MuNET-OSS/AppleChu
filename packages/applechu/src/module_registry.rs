use std::fmt::Display;
use std::sync::Mutex;

use linkme::distributed_slice;

use crate::config::Config;
use crate::system_config::HookModeConfig;
use crate::util::api::Api;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum InitStage {
    Graphics,
    PlatformCore,
    Platform,
    IoHook,
    Device,
    Late,
}

pub struct ModuleDescriptor {
    pub name: &'static str,
    pub stage: InitStage,
    pub order: u16,
    pub enabled: fn(&Config) -> bool,
    pub init: fn(&Api, &Config) -> Result<(), String>,
    pub shutdown: Option<fn()>,
}

#[distributed_slice]
pub static MODULES: [ModuleDescriptor];

static INITIALIZED: Mutex<Vec<&'static ModuleDescriptor>> = Mutex::new(Vec::new());

// 初始化顺序必须满足平台基础设施、设备通信和硬件模拟之间的依赖
// proc_addr、reg_hook 和 iohook 是 Rust 公共实现拆出的基础设施，不属于额外功能
pub trait ModuleInitResult {
    fn into_module_result(self) -> Result<(), String>;
}

impl ModuleInitResult for () {
    fn into_module_result(self) -> Result<(), String> {
        Ok(())
    }
}

impl<E: Display> ModuleInitResult for Result<(), E> {
    fn into_module_result(self) -> Result<(), String> {
        self.map_err(|error| error.to_string())
    }
}

pub fn normalize_init_result<T: ModuleInitResult>(value: T) -> Result<(), String> {
    value.into_module_result()
}

pub fn registered_modules() -> Vec<&'static ModuleDescriptor> {
    let mut modules = MODULES.iter().collect::<Vec<_>>();
    modules.sort_by_key(|module| (module.stage, module.order, module.name));
    modules
}

pub fn init_all(api: &Api, config: &Config) {
    init_matching(api, config, |_| true, false);
}

/// 按 AM Daemon 对应的固定顺序初始化调用方提供的模块
pub fn init_ordered(api: &Api, config: &Config, order: &[&str]) {
    let modules = registered_modules();
    for suffix in order {
        let Some(module) = modules
            .iter()
            .copied()
            .find(|module| module.name.ends_with(suffix))
        else {
            api.log_error(&format!("Required AM Daemon module is missing: {suffix}"));
            unsafe { windows_sys::Win32::System::Threading::ExitProcess(1) };
        };
        if !(module.enabled)(config) {
            continue;
        }
        init_module(api, config, module, true);
    }
}

fn init_matching(api: &Api, config: &Config, include_stage: fn(InitStage) -> bool, fatal: bool) {
    let hook_mode = config.section::<HookModeConfig>();
    let platform = hook_mode.as_ref().is_none_or(|config| config.platform);
    let platform_modules = hook_mode
        .as_ref()
        .is_none_or(|config| config.platform_modules);
    let iohook = hook_mode.as_ref().is_none_or(|config| config.iohook);
    let devices = hook_mode.as_ref().is_none_or(|config| config.devices);

    for module in registered_modules() {
        if !include_stage(module.stage)
            || !stage_enabled(module.stage, platform, platform_modules, iohook, devices)
            || !(module.enabled)(config)
        {
            continue;
        }
        init_module(api, config, module, fatal);
    }

    if !platform {
        api.log_info("Platform compatibility disabled by diagnostics config");
    } else {
        if !platform_modules {
            api.log_info("Platform modules disabled by diagnostics config");
        }
        if !iohook {
            api.log_info("Device I/O compatibility disabled by diagnostics config");
        }
    }
    if !devices {
        api.log_info("Hardware emulation disabled by diagnostics config");
    }
}

fn init_module(api: &Api, config: &Config, module: &'static ModuleDescriptor, fatal: bool) {
    if let Err(error) = (module.init)(api, config) {
        api.log_error(&format!(
            "Module {} failed to initialize: {error}",
            module.name
        ));
        if fatal {
            unsafe { windows_sys::Win32::System::Threading::ExitProcess(1) };
        }
        return;
    }
    if module.shutdown.is_some() {
        if let Ok(mut initialized) = INITIALIZED.lock() {
            initialized.push(module);
        }
    }
}

pub(crate) fn shutdown_all() {
    let modules = match INITIALIZED.lock() {
        Ok(mut modules) => std::mem::take(&mut *modules),
        Err(_) => return,
    };
    for module in modules.into_iter().rev() {
        if let Some(shutdown) = module.shutdown {
            shutdown();
        }
    }
}

fn stage_enabled(
    stage: InitStage,
    platform: bool,
    platform_modules: bool,
    iohook: bool,
    devices: bool,
) -> bool {
    match stage {
        InitStage::Graphics | InitStage::Late => true,
        InitStage::PlatformCore => platform,
        InitStage::Platform => platform && platform_modules,
        InitStage::IoHook => platform && iohook,
        InitStage::Device => devices,
    }
}

#[cfg(test)]
mod tests;
