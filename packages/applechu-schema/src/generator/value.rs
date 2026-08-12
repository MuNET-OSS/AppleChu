use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, ExprMacro, Lit, Token, Type};

use crate::SchemaError;

pub(super) fn schema_type(value_type: &Type) -> Result<String, SchemaError> {
    let Type::Path(path) = value_type else {
        return Err(SchemaError::Invalid("配置字段类型必须是路径".to_owned()));
    };
    let name = path
        .path
        .segments
        .last()
        .map(|part| part.ident.to_string())
        .unwrap_or_default();
    match name.as_str() {
        "bool" => Ok("bool".to_owned()),
        "String" => Ok("string".to_owned()),
        "Vec" => Ok("string_array".to_owned()),
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" => Ok("int".to_owned()),
        _ => Err(SchemaError::Invalid(format!(
            "不支持的配置字段类型: {name}"
        ))),
    }
}

pub(super) fn expression_value(expression: &Expr) -> Result<toml::Value, SchemaError> {
    match expression {
        Expr::Lit(ExprLit {
            lit: Lit::Bool(value),
            ..
        }) => Ok(toml::Value::Boolean(value.value)),
        Expr::Lit(ExprLit {
            lit: Lit::Int(value),
            ..
        }) => value
            .base10_parse::<i64>()
            .map(toml::Value::Integer)
            .map_err(|error| SchemaError::Invalid(error.to_string())),
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(toml::Value::String(value.value())),
        Expr::Lit(ExprLit {
            lit: Lit::Byte(value),
            ..
        }) => Ok(toml::Value::Integer(i64::from(value.value()))),
        Expr::Unary(value) if matches!(value.op, syn::UnOp::Neg(_)) => {
            match expression_value(&value.expr)? {
                toml::Value::Integer(value) => Ok(toml::Value::Integer(-value)),
                _ => Err(SchemaError::Invalid("负号只能用于整数默认值".to_owned())),
            }
        }
        Expr::Cast(value) => expression_value(&value.expr),
        Expr::Call(value) => match (expression_name(&value.func).as_str(), value.args.first()) {
            ("new", None) => Ok(toml::Value::String(String::new())),
            ("from", Some(argument)) => expression_value(argument),
            (name, _) => Err(SchemaError::Invalid(format!("不支持的默认值调用: {name}"))),
        },
        Expr::MethodCall(value) if value.method == "to_owned" => expression_value(&value.receiver),
        Expr::Array(value) => Ok(toml::Value::Array(
            value
                .elems
                .iter()
                .map(expression_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Expr::Path(value) => Err(SchemaError::Invalid(format!(
            "无法静态求值默认值路径 {}，请声明 schema_default",
            value
                .path
                .segments
                .last()
                .map(|part| part.ident.to_string())
                .unwrap_or_default()
        ))),
        Expr::Macro(ExprMacro { mac, .. }) if mac.path.is_ident("vec") => Ok(toml::Value::Array(
            mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
                .map_err(|error| SchemaError::Invalid(error.to_string()))?
                .iter()
                .map(expression_value)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Expr::Paren(value) => expression_value(&value.expr),
        _ => Err(SchemaError::Invalid("不支持的配置默认值表达式".to_owned())),
    }
}

fn expression_name(expression: &Expr) -> String {
    match expression {
        Expr::Path(value) => value
            .path
            .segments
            .last()
            .map(|part| part.ident.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    }
}
