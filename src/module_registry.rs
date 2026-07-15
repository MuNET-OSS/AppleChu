use std::sync::Mutex;

use linkme::distributed_slice;

use crate::config::Config;
use crate::system_config::HookModeConfig;
use crate::util::api::Api;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum InitStage {
    Graphics,
    PlatformCore,
    Platform,
    IoHook,
    Device,
    Late,
}

pub(crate) struct ModuleDescriptor {
    pub name: &'static str,
    pub stage: InitStage,
    pub order: u16,
    pub enabled: fn(&Config) -> bool,
    pub init: fn(&Api, &Config),
    pub shutdown: Option<fn()>,
}

#[distributed_slice]
pub(crate) static MODULES: [ModuleDescriptor];

static INITIALIZED: Mutex<Vec<&'static ModuleDescriptor>> = Mutex::new(Vec::new());

pub(crate) fn registered_modules() -> Vec<&'static ModuleDescriptor> {
    let mut modules = MODULES.iter().collect::<Vec<_>>();
    modules.sort_by_key(|module| (module.stage, module.order, module.name));
    modules
}

pub(crate) fn init_all(api: &Api, config: &Config) {
    let hook_mode = config.section::<HookModeConfig>();
    let platform = hook_mode.as_ref().is_none_or(|config| config.platform);
    let platform_modules = hook_mode
        .as_ref()
        .is_none_or(|config| config.platform_modules);
    let iohook = hook_mode.as_ref().is_none_or(|config| config.iohook);
    let devices = hook_mode.as_ref().is_none_or(|config| config.devices);

    for module in registered_modules() {
        if !stage_enabled(module.stage, platform, platform_modules, iohook, devices)
            || !(module.enabled)(config)
        {
            continue;
        }
        (module.init)(api, config);
        if module.shutdown.is_some() {
            if let Ok(mut initialized) = INITIALIZED.lock() {
                initialized.push(module);
            }
        }
    }

    if !platform {
        api.log_info("platform hooks DISABLED");
    } else {
        if !platform_modules {
            api.log_info("platform modules DISABLED (diag)");
        }
        if !iohook {
            api.log_info("iohook DISABLED (diag)");
        }
    }
    if !devices {
        api.log_info("device emulation DISABLED");
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
