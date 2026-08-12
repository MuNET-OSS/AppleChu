mod document;
pub mod schema;
mod validation;
pub mod value;

pub use document::Config;
pub use schema::{ConfigDiagnostic, ConfigSection, DiagnosticLevel};

#[doc(hidden)]
#[macro_export]
macro_rules! __config_key {
    ($field:ident) => {
        stringify!($field)
    };
    ($field:ident, $key:literal) => {
        $key
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __config_emit_default {
    () => {
        false
    };
    ($value:expr) => {
        $value
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __config_advanced {
    () => {
        false
    };
    ($value:expr) => {
        $value
    };
}

#[macro_export]
macro_rules! config_section {
    (
        $(#[$struct_meta:meta])*
        $vis:vis struct $name:ident => $registration:ident {
            section: $section:literal,
            order: $order:expr,
            default_on: $default_on:expr,
            always_enabled: $always_enabled:expr,
            hidden: $hidden:expr,
            $(aliases: [$($alias:literal),* $(,)?],)?
            $(export: $export:expr,)?
            $(group: $group:literal,)?
            $(community: $community:expr,)?
            $(description: $description:literal,)?
            $(description_en: $description_en:literal,)?
            comment: $comment:literal,
            fields: {
                $(
                    $(#[$field_meta:meta])*
                    $field_vis:vis $field:ident : $ty:ty = $default:expr,
                    $(key: $key:literal,)?
                    $(emit_default: $emit_default:expr,)?
                    $(advanced: $advanced:expr,)?
                    $(schema_type: $schema_type:literal,)?
                    $(schema_default: $schema_default:expr,)?
                    $(min: $min:expr,)?
                    $(max: $max:expr,)?
                    $(options: [$($option:expr),* $(,)?],)?
                    $(description: $field_description:literal,)?
                    $(description_en: $field_description_en:literal,)?
                    comment: $field_comment:literal;
                )*
            }
        }
    ) => {
        $(#[$struct_meta])*
        #[derive(Clone, Debug)]
        $vis struct $name {
            $(
                $(#[$field_meta])*
                $field_vis $field: $ty,
            )*
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    $($field: $default,)*
                }
            }
        }

        impl $name {
            fn parse_config_section(
                table: Option<&toml::Table>,
                diagnostics: &mut Vec<$crate::config::ConfigDiagnostic>,
            ) -> $crate::config::schema::LoadedSection {
                let mut value = Self::default();
                let mut explicit_fields = Vec::new();
                let _ = &mut value;
                let _ = &mut explicit_fields;
                $(
                    let raw = $crate::config::schema::find_key(
                        table,
                        $crate::__config_key!($field $(, $key)?),
                    );
                    explicit_fields.push(raw.is_some());
                    if let Some(raw) = raw {
                        if let Some(parsed) = <$ty as $crate::config::value::ConfigValue>::parse(raw) {
                            value.$field = parsed;
                        } else {
                            diagnostics.push($crate::config::ConfigDiagnostic::warning(format!(
                                "Invalid value or type for {}.{}; using the default",
                                $section, $crate::__config_key!($field $(, $key)?)
                            )));
                        }
                    }
                )*
                $crate::config::schema::warn_unknown_keys(
                    table,
                    $section,
                    &["enable", $($crate::__config_key!($field $(, $key)?),)*],
                    diagnostics,
                );
                $crate::config::schema::LoadedSection::new::<Self>(
                    table,
                    value,
                    explicit_fields,
                    diagnostics,
                )
            }

            fn serialize_config_fields(
                loaded: &$crate::config::schema::LoadedSection,
                output: &mut String,
            ) {
                let Some(value) = loaded.value::<Self>() else {
                    return;
                };
                let mut explicit_fields = loaded.explicit_fields().iter();
                let _ = (&value, &mut explicit_fields, &mut *output);
                $(
                    let explicit = explicit_fields.next().copied().unwrap_or(false);
                    if explicit || !$crate::__config_advanced!($($advanced)?) {
                        $crate::config::schema::append_field_comment(
                            output,
                            $section,
                            $crate::__config_key!($field $(, $key)?),
                            $field_comment,
                        );
                        $crate::config::schema::append_entry(
                            output,
                            $crate::__config_key!($field $(, $key)?),
                            &value.$field,
                            explicit || $crate::__config_emit_default!($($emit_default)?),
                        );
                    }
                )*
            }
        }

        #[linkme::distributed_slice($crate::config::schema::CONFIG_SECTIONS)]
        static $registration: $crate::config::schema::SectionDescriptor =
            $crate::config::schema::SectionDescriptor {
                name: $section,
                order: $order,
                default_on: $default_on,
                always_enabled: $always_enabled,
                hidden: $hidden,
                aliases: &[$($($alias),*)?],
                comment: $comment,
                type_id: std::any::TypeId::of::<$name>,
                parse: $name::parse_config_section,
                serialize_fields: $name::serialize_config_fields,
                field_keys: &[$($crate::__config_key!($field $(, $key)?),)*],
            };

        impl $crate::config::ConfigSection for $name {
            fn descriptor() -> &'static $crate::config::schema::SectionDescriptor {
                &$registration
            }
        }
    };
}

#[cfg(test)]
mod tests;
