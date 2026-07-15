use crate::config::value::ConfigValue;
use crate::config::Config;

#[derive(Clone, Debug)]
pub(crate) enum CabinetMode {
    Sp,
    Cvt,
}

impl ConfigValue for CabinetMode {
    fn parse(value: &toml::Value) -> Option<Self> {
        match value.as_str()?.trim().to_ascii_lowercase().as_str() {
            "sp" => Some(Self::Sp),
            "cvt" => Some(Self::Cvt),
            _ => None,
        }
    }

    fn to_toml(&self) -> toml::Value {
        toml::Value::String(
            match self {
                Self::Sp => "SP",
                Self::Cvt => "CVT",
            }
            .to_owned(),
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RefreshRate {
    Hz60,
    Hz120,
}

impl ConfigValue for RefreshRate {
    fn parse(value: &toml::Value) -> Option<Self> {
        match value.as_integer()? {
            60 => Some(Self::Hz60),
            120 => Some(Self::Hz120),
            _ => None,
        }
    }

    fn to_toml(&self) -> toml::Value {
        toml::Value::Integer(match self {
            Self::Hz60 => 60,
            Self::Hz120 => 120,
        })
    }
}

crate::config_section! {
    pub(crate) struct SystemConfig => SYSTEM_CONFIG_SECTION {
        section: "System",
        order: 10,
        default_enabled: true,
        always_enabled: true,
        hidden: false,
        comment: "系统设置",
        fields: {
            pub enable_console: bool = true,
            key: "EnableConsole",
            comment: "是否创建新的控制台窗口";
            pub lan_slave: bool = false,
            key: "LanSlave",
            comment: "店内联机从机模式";
            pub mode: CabinetMode = CabinetMode::Sp,
            key: "Mode",
            comment: "机台模式：SP 或 CVT";
            pub refresh_rate: RefreshRate = RefreshRate::Hz60,
            key: "RefreshRate",
            comment: "显示器刷新率：60 或 120";
        }
    }
}

impl SystemConfig {
    pub fn is_sp_mode(&self) -> bool {
        matches!(self.mode, CabinetMode::Sp)
    }

    pub fn dipsw(&self) -> [bool; 3] {
        [
            !self.lan_slave,
            matches!(self.refresh_rate, RefreshRate::Hz60),
            matches!(self.refresh_rate, RefreshRate::Hz60),
        ]
    }
}

pub(crate) fn is_sp_mode(config: &Config) -> bool {
    config
        .section::<SystemConfig>()
        .is_some_and(|config| config.is_sp_mode())
}

crate::config_section! {
    pub(crate) struct HookModeConfig => HOOK_MODE_CONFIG_SECTION {
        section: "HookMode",
        order: 900,
        default_enabled: true,
        always_enabled: true,
        hidden: true,
        comment: "内部 Hook 诊断开关",
        fields: {
            pub platform: bool = true,
            comment: "启用平台 Hook";
            pub devices: bool = true,
            comment: "启用设备模拟";
            pub platform_modules: bool = true,
            key: "platformModules",
            comment: "启用平台模块";
            pub iohook: bool = true,
            comment: "启用 IO Hook";
        }
    }
}
