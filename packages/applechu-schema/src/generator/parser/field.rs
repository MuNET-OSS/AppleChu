use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitBool, LitStr, Token, Type};

pub(crate) struct FieldDecl {
    pub name: Ident,
    pub value_type: Type,
    pub default: Expr,
    pub key: Option<LitStr>,
    pub emit_default: bool,
    pub advanced: bool,
    pub schema_type: Option<LitStr>,
    pub schema_default: Option<Expr>,
    pub min: Option<Expr>,
    pub max: Option<Expr>,
    pub options: Vec<Expr>,
    pub description: Option<LitStr>,
    pub description_en: Option<LitStr>,
    pub comment: Option<LitStr>,
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
        let mut field = Self {
            name,
            value_type,
            default,
            key: None,
            emit_default: false,
            advanced: false,
            schema_type: None,
            schema_default: None,
            min: None,
            max: None,
            options: Vec::new(),
            description: None,
            description_en: None,
            comment: None,
        };
        if input.peek(Token![;]) {
            input.parse::<Token![;]>()?;
            return Ok(field);
        }
        input.parse::<Token![,]>()?;
        loop {
            let metadata: Ident = input.parse()?;
            input.parse::<Token![:]>()?;
            match metadata.to_string().as_str() {
                "key" => field.key = Some(input.parse()?),
                "emit_default" => field.emit_default = input.parse::<LitBool>()?.value,
                "advanced" => field.advanced = input.parse::<LitBool>()?.value,
                "schema_type" => field.schema_type = Some(input.parse()?),
                "schema_default" => field.schema_default = Some(input.parse()?),
                "min" => field.min = Some(input.parse()?),
                "max" => field.max = Some(input.parse()?),
                "options" => {
                    let content;
                    syn::bracketed!(content in input);
                    field.options = content
                        .parse_terminated(Expr::parse, Token![,])?
                        .into_iter()
                        .collect();
                }
                "description" => field.description = Some(input.parse()?),
                "description_en" => field.description_en = Some(input.parse()?),
                "comment" => field.comment = Some(input.parse()?),
                _ => return Err(syn::Error::new(metadata.span(), "unknown field metadata")),
            }
            if input.peek(Token![;]) {
                input.parse::<Token![;]>()?;
                return Ok(field);
            }
            input.parse::<Token![,]>()?;
        }
    }
}
