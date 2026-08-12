use std::fs;
use std::path::{Path, PathBuf};

use syn::parse::{Parse, ParseStream};
use syn::{braced, Ident, LitBool, LitInt, LitStr, Token};

use crate::SchemaError;

mod field;

pub(super) use field::FieldDecl;

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
                "aliases" => {
                    let aliases_content;
                    syn::bracketed!(aliases_content in content);
                    let _ = aliases_content
                        .parse_terminated(|input| input.parse::<LitStr>(), Token![,])?;
                }
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
