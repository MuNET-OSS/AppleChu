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

crate::config_section! {
    pub(crate) struct SystemConfig => SYSTEM_CONFIG_SECTION {
        section: "System",
        order: 10,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "系统设置",
        fields: {
            pub enable_console: bool = true,
            description: "关闭后沿用启动进程的标准输出流",
            description_en: "Reuse the launcher's standard output stream when disabled",
            comment: "是否创建新的控制台窗口";
            pub lan_slave: bool = false,
            comment: "店内联机从机模式";
            pub mode: CabinetMode = CabinetMode::Sp,
            schema_type: "string",
            schema_default: "SP",
            options: ["SP", "CVT"],
            comment: "机台模式：SP（120Hz）或 CVT（60Hz）";
        }
    }
}

impl SystemConfig {
    pub fn is_sp_mode(&self) -> bool {
        matches!(self.mode, CabinetMode::Sp)
    }

    pub fn dipsw(&self) -> [bool; 3] {
        let is_cvt = matches!(self.mode, CabinetMode::Cvt);
        [
            !self.lan_slave,
            is_cvt,
            // CVT 同时要求 60Hz 与机台模式拨码，避免生成硬件不存在的组合
            is_cvt,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cabinet_mode_selects_its_fixed_refresh_rate() {
        // Given: SP 与 CVT 两种机台模式。
        let sp = SystemConfig::default();
        let cvt = SystemConfig {
            lan_slave: true,
            mode: CabinetMode::Cvt,
            ..SystemConfig::default()
        };

        // When: 系统生成机台拨码开关。
        let sp_dipsw = sp.dipsw();
        let cvt_dipsw = cvt.dipsw();

        // Then: SP 固定 120Hz，CVT 固定 60Hz。
        assert_eq!(sp_dipsw, [true, false, false]);
        assert_eq!(cvt_dipsw, [false, true, true]);
    }
}

pub(crate) fn is_sp_mode(config: &Config) -> bool {
    config
        .section::<SystemConfig>()
        .filter(|config| config.enabled)
        .is_none_or(|config| config.is_sp_mode())
}

crate::config_section! {
    pub(crate) struct HookModeConfig => HOOK_MODE_CONFIG_SECTION {
        section: "HookMode",
        order: 900,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "内部 Hook 诊断开关",
        fields: {
            pub platform: bool = true,
            comment: "启用平台 Hook";
            pub devices: bool = true,
            comment: "启用设备模拟";
            pub platform_modules: bool = true,
            comment: "启用平台模块";
            pub iohook: bool = true,
            comment: "启用 IO Hook";
        }
    }
}
