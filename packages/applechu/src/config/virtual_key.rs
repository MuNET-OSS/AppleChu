use super::value::ConfigValue;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualKey(u8);

impl VirtualKey {
    pub const fn new(code: u8) -> Self {
        Self(code)
    }

    pub const fn code(self) -> i32 {
        self.0 as i32
    }

    fn parse_str(value: &str) -> Option<Self> {
        let key = value.trim();
        if let Some(hex) = key.strip_prefix("0x").or_else(|| key.strip_prefix("0X")) {
            return u8::from_str_radix(hex, 16)
                .ok()
                .filter(|code| *code != 0)
                .map(Self);
        }
        if let Ok(code) = key.parse::<u8>() {
            return (code != 0).then_some(Self(code));
        }

        let normalized = key.replace([' ', '-', '_'], "").to_ascii_uppercase();
        let normalized = normalized.strip_prefix("VK").unwrap_or(&normalized);
        let code = match normalized {
            "BACKSPACE" | "BACK" => 0x08,
            "TAB" => 0x09,
            "ENTER" | "RETURN" => 0x0D,
            "SHIFT" => 0x10,
            "CTRL" | "CONTROL" => 0x11,
            "ALT" | "MENU" => 0x12,
            "PAUSE" => 0x13,
            "CAPSLOCK" | "CAPITAL" => 0x14,
            "ESC" | "ESCAPE" => 0x1B,
            "SPACE" => 0x20,
            "PAGEUP" | "PRIOR" => 0x21,
            "PAGEDOWN" | "NEXT" => 0x22,
            "END" => 0x23,
            "HOME" => 0x24,
            "LEFT" => 0x25,
            "UP" => 0x26,
            "RIGHT" => 0x27,
            "DOWN" => 0x28,
            "INSERT" => 0x2D,
            "DELETE" | "DEL" => 0x2E,
            key if key.len() == 1 && key.is_ascii() => key.as_bytes()[0],
            key if key.starts_with('F') => {
                let number = key[1..].parse::<u8>().ok()?;
                (1..=24).contains(&number).then_some(0x70 + number - 1)?
            }
            _ => return None,
        };
        Some(Self(code))
    }
}

impl ConfigValue for VirtualKey {
    fn parse(value: &toml::Value) -> Option<Self> {
        match value {
            toml::Value::Integer(code) => {
                u8::try_from(*code).ok().filter(|code| *code != 0).map(Self)
            }
            toml::Value::String(value) => Self::parse_str(value),
            toml::Value::Float(_)
            | toml::Value::Boolean(_)
            | toml::Value::Datetime(_)
            | toml::Value::Array(_)
            | toml::Value::Table(_) => None,
        }
    }

    fn to_toml(&self) -> toml::Value {
        toml::Value::Integer(i64::from(self.0))
    }

    fn to_toml_literal(&self) -> String {
        format!("0x{:02X}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualKey;
    use crate::config::value::ConfigValue;

    #[test]
    fn named_and_numeric_values_parse_to_the_same_key() {
        // Given: 同一个键位的名称、十六进制文本和整数表示。
        let values = [
            toml::Value::String("F2".to_owned()),
            toml::Value::String("0x71".to_owned()),
            toml::Value::Integer(0x71),
        ];

        // When: 输入在配置边界被解析。
        let parsed = values.map(|value| VirtualKey::parse(&value));

        // Then: 三种表示得到同一个强类型键码。
        assert_eq!(parsed, [Some(VirtualKey::new(0x71)); 3]);
    }

    #[test]
    fn invalid_virtual_keys_are_rejected() {
        // Given: 零值、超范围值和未知名称。
        let values = [
            toml::Value::Integer(0),
            toml::Value::Integer(0x100),
            toml::Value::String("F25".to_owned()),
        ];

        // When: 输入在配置边界被解析。
        let parsed = values.map(|value| VirtualKey::parse(&value));

        // Then: 无效键码不会进入运行时。
        assert_eq!(parsed, [None, None, None]);
    }
}
