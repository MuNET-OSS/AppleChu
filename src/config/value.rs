pub trait ConfigValue: Clone + Send + Sync + 'static {
    fn parse(value: &toml::Value) -> Option<Self>;
    fn to_toml(&self) -> toml::Value;
}

impl ConfigValue for bool {
    fn parse(value: &toml::Value) -> Option<Self> {
        match value {
            toml::Value::Boolean(value) => Some(*value),
            toml::Value::Integer(value) => Some(*value != 0),
            toml::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            },
            toml::Value::Float(_)
            | toml::Value::Datetime(_)
            | toml::Value::Array(_)
            | toml::Value::Table(_) => None,
        }
    }

    fn to_toml(&self) -> toml::Value {
        toml::Value::Boolean(*self)
    }
}

macro_rules! integer_config_value {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ConfigValue for $ty {
                fn parse(value: &toml::Value) -> Option<Self> {
                    value.as_integer().and_then(|value| Self::try_from(value).ok())
                }

                fn to_toml(&self) -> toml::Value {
                    toml::Value::Integer(i64::from(*self))
                }
            }
        )*
    };
}

integer_config_value!(i8, i16, i32, u8, u16, u32);

impl ConfigValue for i64 {
    fn parse(value: &toml::Value) -> Option<Self> {
        value.as_integer()
    }

    fn to_toml(&self) -> toml::Value {
        toml::Value::Integer(*self)
    }
}

impl ConfigValue for String {
    fn parse(value: &toml::Value) -> Option<Self> {
        value.as_str().map(ToOwned::to_owned)
    }

    fn to_toml(&self) -> toml::Value {
        toml::Value::String(self.clone())
    }
}
