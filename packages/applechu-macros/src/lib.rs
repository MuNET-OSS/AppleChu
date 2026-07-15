use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    parse_macro_input, Error, FnArg, Ident, ItemFn, LitInt, Path, Result, Token, Type, TypePath,
};

struct ConfigSectionArgs {
    config: Option<TypePath>,
    stage: Ident,
    order: u16,
    condition: Option<Path>,
    shutdown: Option<Path>,
}

impl Parse for ConfigSectionArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut config = None;
        let mut stage = None;
        let mut order = None;
        let mut condition = None;
        let mut shutdown = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "config" => config = Some(input.parse()?),
                "stage" => stage = Some(input.parse()?),
                "order" => {
                    let value: LitInt = input.parse()?;
                    order = Some(value.base10_parse()?);
                }
                "condition" => condition = Some(input.parse()?),
                "shutdown" => shutdown = Some(input.parse()?),
                _ => return Err(Error::new(key.span(), "不支持的 config_section 参数")),
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self {
            config,
            stage: stage.ok_or_else(|| Error::new(Span::call_site(), "缺少 stage"))?,
            order: order.ok_or_else(|| Error::new(Span::call_site(), "缺少 order"))?,
            condition,
            shutdown,
        })
    }
}

#[proc_macro_attribute]
pub fn config_section(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ConfigSectionArgs);
    let function = parse_macro_input!(item as ItemFn);
    expand_config_section(args, function)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand_config_section(
    args: ConfigSectionArgs,
    function: ItemFn,
) -> Result<proc_macro2::TokenStream> {
    let name = &function.sig.ident;
    let wrapper = format_ident!("__applechu_init_{name}");
    let enabled = format_ident!("__applechu_enabled_{name}");
    let registration = format_ident!("__APPLECHU_MODULE_{}", name.to_string().to_uppercase());
    let stage = args.stage;
    let order = args.order;
    let argument_count = function.sig.inputs.len();
    let config_type = match args.config {
        Some(config_type) => Some(config_type),
        None => infer_config_type(&function)?,
    };

    let condition = args
        .condition
        .map_or_else(|| quote!(true), |condition| quote!((#condition)(config)));

    let (config_gate, call) = match config_type {
        Some(config_type) => {
            let call = match argument_count {
                2 => quote!(#name(api, &section)),
                3 => quote!(#name(api, config, &section)),
                _ => {
                    return Err(Error::new_spanned(
                        &function.sig,
                        "带 config 的 init 必须接收 (api, section) 或 (api, config, section)",
                    ));
                }
            };
            (
                quote!(
                    config
                        .section::<#config_type>()
                        .is_some_and(|section| section.enabled)
                ),
                quote!(
                    let Some(section) = config.section::<#config_type>() else {
                        return;
                    };
                    #call;
                ),
            )
        }
        None => {
            let call = match argument_count {
                1 => quote!(#name(api)),
                2 => quote!(#name(api, config)),
                _ => {
                    return Err(Error::new_spanned(
                        &function.sig,
                        "无 config 的 init 必须接收 (api) 或 (api, config)",
                    ));
                }
            };
            (quote!(true), quote!(#call;))
        }
    };

    let shutdown = args
        .shutdown
        .map_or_else(|| quote!(None), |shutdown| quote!(Some(#shutdown)));

    Ok(quote! {
        #function

        fn #enabled(config: &crate::config::Config) -> bool {
            #condition && #config_gate
        }

        fn #wrapper(api: &crate::util::api::Api, config: &crate::config::Config) {
            #call
        }

        #[linkme::distributed_slice(crate::module_registry::MODULES)]
        static #registration: crate::module_registry::ModuleDescriptor =
            crate::module_registry::ModuleDescriptor {
                name: concat!(module_path!(), "::", stringify!(#name)),
                stage: crate::module_registry::InitStage::#stage,
                order: #order,
                enabled: #enabled,
                init: #wrapper,
                shutdown: #shutdown,
            };
    })
}

fn infer_config_type(function: &ItemFn) -> Result<Option<TypePath>> {
    let inputs = function.sig.inputs.iter().collect::<Vec<_>>();
    match inputs.len() {
        1 => Ok(None),
        2 => {
            let config_type = referenced_type_path(inputs[1])?;
            if config_type
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Config")
            {
                Ok(None)
            } else {
                Ok(Some(config_type))
            }
        }
        3 => Ok(Some(referenced_type_path(inputs[2])?)),
        _ => Err(Error::new_spanned(
            &function.sig,
            "init 必须接收 (api)、(api, config/section) 或 (api, config, section)",
        )),
    }
}

fn referenced_type_path(argument: &FnArg) -> Result<TypePath> {
    let FnArg::Typed(argument) = argument else {
        return Err(Error::new_spanned(argument, "init 不支持 self 参数"));
    };
    let Type::Reference(reference) = argument.ty.as_ref() else {
        return Err(Error::new_spanned(&argument.ty, "配置参数必须是引用"));
    };
    let Type::Path(config_type) = reference.elem.as_ref() else {
        return Err(Error::new_spanned(&reference.elem, "无法推导配置类型"));
    };
    Ok(config_type.clone())
}
