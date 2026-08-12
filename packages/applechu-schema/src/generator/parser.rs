use std::fs;
use std::path::{Path, PathBuf};

use syn::parse::{Parse, ParseStream};
use syn::{braced, Expr, Ident, LitBool, LitInt, LitStr, Token, Type};

use crate::SchemaError;

pub(super) struct SectionDecl {
    pub name: LitStr,
    pub order: LitInt,
    pub default_on: LitBool,
    pub always_enabled: LitBool,
    pub hidden: LitBool,
    pub export: Option<bool>,
    pub group: Option<LitStr>,
    pub community: bool,
    pub description: Option<LitStr>,
    pub description_en: Option<LitStr>,
    pub comment: LitStr,
    pub fields: Vec<FieldDecl>,
}

pub(super) struct FieldDecl {
    pub name: Ident,
    pub value_type: Type,
    pub default: Expr,
    pub key: Option<LitStr>,
    pub emit_default: bool,
    pub schema_type: Option<LitStr>,
    pub schema_default: Option<Expr>,
    pub min: Option<Expr>,
    pub max: Option<Expr>,
    pub options: Vec<Expr>,
    pub description: Option<LitStr>,
    pub description_en: Option<LitStr>,
    pub comment: LitStr,
}

impl Parse for SectionDecl {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let _: syn::Visibility = input.parse()?;
        input.parse::<Token![struct]>()?;
        input.parse::<Ident>()?;
        input.parse::<Token![=>]>()?;
        input.parse::<Ident>()?;
        let content;
        braced!(content in input);
        let name = parse_named(&content, "section")?;
        let order = parse_named(&content, "order")?;
        let default_on = parse_named(&content, "default_on")?;
        let always_enabled = parse_named(&content, "always_enabled")?;
        let hidden = parse_named(&content, "hidden")?;
        let mut export = None;
        let mut group = None;
        let mut community = false;
        let mut description = None;
        let mut description_en = None;
        let comment = loop {
            let metadata: Ident = content.parse()?;
            content.parse::<Token![:]>()?;
            match metadata.to_string().as_str() {
                "export" => export = Some(content.parse::<LitBool>()?.value),
                "group" => group = Some(content.parse()?),
                "community" => community = content.parse::<LitBool>()?.value,
                "description" => description = Some(content.parse()?),
                "description_en" => description_en = Some(content.parse()?),
                "comment" => {
                    let comment = content.parse()?;
                    content.parse::<Token![,]>()?;
                    break comment;
                }
                _ => return Err(syn::Error::new(metadata.span(), "unknown section metadata")),
            }
            content.parse::<Token![,]>()?;
        };
        let fields_name: Ident = content.parse()?;
        if fields_name != "fields" {
            return Err(syn::Error::new(fields_name.span(), "expected fields"));
        }
        content.parse::<Token![:]>()?;
        let fields_content;
        braced!(fields_content in content);
        let mut fields = Vec::new();
        while !fields_content.is_empty() {
            fields.push(fields_content.parse()?);
        }
        Ok(Self {
            name,
            order,
            default_on,
            always_enabled,
            hidden,
            export,
            group,
            community,
            description,
            description_en,
            comment,
            fields,
        })
    }
}

impl Parse for FieldDecl {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let _: Vec<syn::Attribute> = input.call(syn::Attribute::parse_outer)?;
        let _: syn::Visibility = input.parse()?;
        let name = input.parse()?;
        input.parse::<Token![:]>()?;
        let value_type = input.parse()?;
        input.parse::<Token![=]>()?;
        let default = input.parse()?;
        input.parse::<Token![,]>()?;
        let mut key = None;
        let mut emit_default = false;
        let mut schema_type = None;
        let mut schema_default = None;
        let mut min = None;
        let mut max = None;
        let mut options = Vec::new();
        let mut description = None;
        let mut description_en = None;
        loop {
            let metadata: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            match metadata.to_string().as_str() {
                "key" => key = Some(input.parse()?),
                "emit_default" => emit_default = input.parse::<LitBool>()?.value,
                "schema_type" => schema_type = Some(input.parse()?),
                "schema_default" => schema_default = Some(input.parse()?),
                "min" => min = Some(input.parse()?),
                "max" => max = Some(input.parse()?),
                "options" => {
                    let content;
                    syn::bracketed!(content in input);
                    options = content
                        .parse_terminated(Expr::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "description" => description = Some(input.parse()?),
                "description_en" => description_en = Some(input.parse()?),
                "comment" => {
                    let comment = input.parse()?;
                    input.parse::<Token![;]>()?;
                    return Ok(Self {
                        name,
                        value_type,
                        default,
                        key,
                        emit_default,
                        schema_type,
                        schema_default,
                        min,
                        max,
                        options,
                        description,
                        description_en,
                        comment,
                    });
                }
                _ => return Err(syn::Error::new(metadata.span(), "unknown field metadata")),
            }
            input.parse::<Token![,]>()?;
        }
    }
}

fn parse_named<T: Parse>(input: ParseStream<'_>, expected: &str) -> syn::Result<T> {
    let name: Ident = input.parse()?;
    if name != expected {
        return Err(syn::Error::new(name.span(), format!("expected {expected}")));
    }
    input.parse::<Token![:]>()?;
    let value = input.parse()?;
    input.parse::<Token![,]>()?;
    Ok(value)
}

pub(super) fn parse_directory(root: &Path) -> Result<Vec<SectionDecl>, SchemaError> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files)?;
    let mut sections = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path).map_err(|error| {
            SchemaError::Invalid(format!("读取 {} 失败: {error}", path.display()))
        })?;
        sections.extend(parse_sections(&source, &path)?);
    }
    Ok(sections)
}

fn collect_rust_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), SchemaError> {
    for entry in fs::read_dir(root)
        .map_err(|error| SchemaError::Invalid(format!("读取 {} 失败: {error}", root.display())))?
    {
        let path = entry
            .map_err(|error| SchemaError::Invalid(error.to_string()))?
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "tests") {
                continue;
            }
            collect_rust_files(&path, output)?;
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_none_or(|name| name != "tests.rs")
        {
            output.push(path);
        }
    }
    Ok(())
}

fn parse_sections(source: &str, path: &Path) -> Result<Vec<SectionDecl>, SchemaError> {
    let file = syn::parse_file(source)
        .map_err(|error| SchemaError::Invalid(format!("解析 {} 失败: {error}", path.display())))?;
    file.items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Macro(item)
                if item
                    .mac
                    .path
                    .segments
                    .last()
                    .is_some_and(|part| part.ident == "config_section") =>
            {
                Some(syn::parse2(item.mac.tokens).map_err(|error| {
                    SchemaError::Invalid(format!(
                        "解析 {} 的 config_section 失败: {error}",
                        path.display()
                    ))
                }))
            }
            _ => None,
        })
        .collect()
}
