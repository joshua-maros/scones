use inflector::Inflector;
use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use std::collections::HashMap;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{
    braced, parenthesized, parse_quote, Error, Expr, Fields, Ident, ItemStruct, Token, Type,
    Visibility,
};

#[derive(Clone)]
struct FieldInfo<'a> {
    ident: Ident,
    ty: &'a Type,
    custom_init: HashMap<String, TokenStream2>,
    default_init: Option<TokenStream2>,
}

enum ConstructorParam {
    /// A parameter which directly corresponds to a specific field.
    Field(Ident),
    /// A parameter which is custom and will be used to initialize other fields.
    Custom(Ident, Type),
    /// A stand-in for any Field parameters not explicitly specified.
    Ellipses,
}

impl Parse for ConstructorParam {
    fn parse(input: ParseStream) -> syn::parse::Result<Self> {
        if input.peek(Token![.]) && input.peek2(Token![.]) {
            let _: Token![.] = input.parse()?;
            let _: Token![.] = input.parse()?;
            Ok(Self::Ellipses)
        } else {
            let name: Ident = input.parse()?;
            if input.peek(Token![:]) {
                let _: Token![:] = input.parse()?;
                let ty: Type = input.parse()?;
                Ok(Self::Custom(name, ty))
            } else {
                Ok(Self::Field(name))
            }
        }
    }
}

struct ConstructorInfo {
    vis: Visibility,
    name: Ident,
    params: Vec<ConstructorParam>,
}

impl Parse for ConstructorInfo {
    fn parse(input: ParseStream) -> syn::parse::Result<Self> {
        let vis: Visibility = if input.fork().parse::<Visibility>().is_ok() {
            input.parse().unwrap()
        } else {
            parse_quote! { pub }
        };
        let (name, params): (Ident, _) = if input.peek(Token![fn]) {
            let _: Token![fn] = input.parse()?;
            let name: Ident = input.parse()?;
            let params = if input.is_empty() {
                vec![ConstructorParam::Ellipses]
            } else {
                let content;
                parenthesized!(content in input);
                let param_list = content.parse_terminated::<_, Comma>(ConstructorParam::parse)?;
                param_list.into_iter().collect()
            };
            (name, params)
        } else {
            (parse_quote!(new), vec![ConstructorParam::Ellipses])
        };
        Ok(Self { vis, name, params })
    }
}

fn make_constructor_args(
    constructor_name: &str,
    param_info: &[ConstructorParam],
    fields: &[FieldInfo],
) -> TokenStream2 {
    let mut param_impls = Vec::new();
    // Stores fields that must be in the parameters of the constructor but the user has not
    // yet explicitly specified where in the parameter list they should go.
    let mut remaining_fields: Vec<_> = fields
        .iter()
        .cloned()
        .filter(|e| !e.custom_init.contains_key(constructor_name) && e.default_init.is_none())
        .collect();
    // If we do not encounter an ellipses, then just insert the extra parameters at the end of the
    // signature.
    let mut remaining_fields_insertion_index = param_info.len();
    for param in param_info {
        match param {
            ConstructorParam::Field(field_name) => {
                let mut success = false;
                for (index, field) in remaining_fields.iter().enumerate() {
                    if &field.ident == field_name {
                        let field = remaining_fields.remove(index);
                        let name = field.ident;
                        let ty = &field.ty;
                        param_impls.push(quote! {
                            #name: #ty
                        });
                        success = true;
                        break;
                    }
                }
                if !success {
                    for field in fields {
                        if &field.ident == field_name {
                            let name = field.ident.clone();
                            let ty = &field.ty;
                            param_impls.push(quote! {
                                #name: #ty
                            });
                            success = true;
                            break;
                        }
                    }
                }
                if !success {
                    return Error::new_spanned(
                        field_name,
                        concat!(
                            "Could not find a field with this name ",
                            "(or it was used earlier in the constructor)"
                        ),
                    )
                    .to_compile_error();
                }
            }
            ConstructorParam::Custom(name, ty) => {
                param_impls.push(quote! {
                    #name: #ty
                });
            }
            ConstructorParam::Ellipses => {
                remaining_fields_insertion_index = param_impls.len();
            }
        }
    }
    for field in remaining_fields {
        let name = field.ident;
        let ty = &field.ty;
        param_impls.insert(
            remaining_fields_insertion_index,
            quote! {
                #name: #ty
            },
        );
        remaining_fields_insertion_index += 1;
    }
    quote! {
        #(#param_impls),*
    }
}

fn make_constructor_impl(info: ConstructorInfo, fields: &[FieldInfo]) -> TokenStream2 {
    let vis = info.vis;
    let name = info.name;
    let name_str = name.to_string();
    let params = make_constructor_args(&name_str, &info.params[..], fields);
    let mut initializers = Vec::new();
    for field in fields {
        let ident = &field.ident;
        let init = field
            .custom_init
            .get(&name_str)
            .or(field.default_init.as_ref())
            .cloned()
            .unwrap_or(quote! { #ident });
        initializers.push(quote! {
            #ident: #init
        });
    }
    quote! {
        #vis fn #name (#params) -> Self {
            Self {
                #(#initializers),*
            }
        }
    }
}

struct ValueBody {
    expr: Expr,
    for_constructor: Option<String>,
}

impl Parse for ValueBody {
    fn parse(input: ParseStream) -> syn::parse::Result<Self> {
        let interior;
        parenthesized!(interior in input);
        let expr: Expr = interior.parse()?;
        let for_constructor = if interior.is_empty() {
            None
        } else {
            let _: Token![for] = interior.parse()?;
            let name: Ident = interior.parse()?;
            Some(name.to_string())
        };
        Ok(Self {
            expr,
            for_constructor,
        })
    }
}

#[proc_macro_attribute]
pub fn make_constructor(attr: TokenStream, item: TokenStream) -> TokenStream {
    let constructor_info: ConstructorInfo = syn::parse_macro_input!(attr);
    let mut constructors = vec![constructor_info];

    let mut struct_def: ItemStruct = syn::parse_macro_input!(item);
    let struct_name = &struct_def.ident;
    let fields = if let Fields::Named(fields) = &mut struct_def.fields {
        &mut fields.named
    } else {
        return Error::new_spanned(
            &struct_def,
            "make_constructor currently only works on structs with named fields.",
        )
        .to_compile_error()
        .into();
    };
    let mut field_infos = Vec::new();
    for field in fields {
        let ident = field.ident.clone().unwrap();
        let mut condemned_indexes = Vec::new();
        let mut custom_init = HashMap::new();
        let mut default_init = None;
        for (index, attr) in field.attrs.iter().enumerate() {
            if attr.path.is_ident("value") {
                condemned_indexes.push(index);
                let tokens = attr.tokens.clone().into();
                let vb: ValueBody = syn::parse_macro_input!(tokens);
                let expr = vb.expr;
                let initializer = quote! { #expr };
                if let Some(for_constructor) = vb.for_constructor {
                    custom_init.insert(for_constructor, initializer);
                } else {
                    default_init = Some(initializer);
                }
            }
        }
        condemned_indexes.reverse();
        for index in condemned_indexes {
            field.attrs.remove(index);
        }
        field_infos.push(FieldInfo {
            ident,
            ty: &field.ty,
            custom_init,
            default_init,
        });
    }

    let mut constructor_defs = Vec::new();
    for cons in constructors {
        constructor_defs.push(make_constructor_impl(cons, &field_infos[..]));
    }

    (quote! {
        #struct_def
        impl #struct_name {
            #(#constructor_defs)*
        }
    })
    .into()
}
