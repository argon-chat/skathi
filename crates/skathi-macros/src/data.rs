use proc_macro2::{Span, TokenStream, TokenTree};
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Item, Path, Token};

fn derived(attrs: &[Attribute]) -> Vec<String> {
    let mut names = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        if let Ok(paths) = attr.parse_args_with(Punctuated::<Path, Token![,]>::parse_terminated) {
            names.extend(
                paths
                    .iter()
                    .filter_map(|p| p.segments.last())
                    .map(|s| s.ident.to_string()),
            );
        }
    }
    names
}

/// Whether some `#[tool(...)]` already names its crate.
fn names_crate(attrs: &[Attribute], tool: &str) -> bool {
    attrs.iter().filter(|a| a.path().is_ident(tool)).any(|a| {
        a.meta.require_list().is_ok_and(|list| {
            list.tokens
                .clone()
                .into_iter()
                .any(|t| matches!(t, TokenTree::Ident(i) if i == "crate"))
        })
    })
}

pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut krate: Option<Path> = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("crate") {
            krate = Some(meta.value()?.parse()?);
            Ok(())
        } else {
            Err(meta.error("expected `crate = path` or nothing"))
        }
    });
    if let Err(e) = syn::parse::Parser::parse2(parser, attr) {
        let error = e.to_compile_error();
        return quote!(#error #item);
    }
    let mut item = match syn::parse2::<Item>(item.clone()) {
        Ok(item @ (Item::Struct(_) | Item::Enum(_))) => item,
        _ => {
            let error = syn::Error::new(
                Span::call_site(),
                "#[skathi::data] goes on a struct or an enum",
            )
            .to_compile_error();
            return quote!(#error #item);
        }
    };
    let krate = crate::crate_path(krate);
    let direct = crate::direct::expand(&item, &krate);
    let attrs = match &mut item {
        Item::Struct(s) => &mut s.attrs,
        Item::Enum(e) => &mut e.attrs,
        _ => unreachable!(),
    };
    let has = derived(attrs);
    let has = |name: &str| has.iter().any(|h| h == name);
    let mut derives = Vec::new();
    if !has("Serialize") {
        derives.push(quote!(#krate::serde::Serialize));
    }
    if !has("Deserialize") {
        derives.push(quote!(#krate::serde::Deserialize));
    }
    let types = cfg!(feature = "types");
    if types && !has("Type") {
        derives.push(quote!(#krate::specta::Type));
    }

    // Derives first, then the item's own attributes, then the crate paths: a helper attribute
    // after every derive that might read it.
    let mut out: Vec<Attribute> = Vec::new();
    if !derives.is_empty() {
        out.push(syn::parse_quote!(#[derive(#(#derives),*)]));
    }
    out.append(attrs);
    if !names_crate(&out, "serde") {
        let path = quote!(#krate::serde).to_string().replace(' ', "");
        out.push(syn::parse_quote!(#[serde(crate = #path)]));
    }
    if types && !names_crate(&out, "specta") {
        out.push(syn::parse_quote!(#[specta(crate = #krate::specta)]));
    }
    *attrs = out;
    match &mut item {
        Item::Struct(s) => s.fields.iter_mut().for_each(|f| bytes(f, &krate, types)),
        Item::Enum(e) => e
            .variants
            .iter_mut()
            .flat_map(|v| v.fields.iter_mut())
            .for_each(|f| bytes(f, &krate, types)),
        _ => {}
    }
    quote!(#item #direct)
}

/// Whether some `#[tool(...)]` on a field already says one of `words`.
fn says(attrs: &[Attribute], tool: &str, words: &[&str]) -> bool {
    attrs.iter().filter(|a| a.path().is_ident(tool)).any(|a| {
        a.meta.require_list().is_ok_and(|list| {
            list.tokens
                .clone()
                .into_iter()
                .any(|t| matches!(t, TokenTree::Ident(i) if words.iter().any(|w| i == w)))
        })
    })
}

/// A `Vec<u8>` field is a `Uint8Array` in JS, unless the field says how it is (de)serialized.
fn bytes(field: &mut syn::Field, krate: &TokenStream, types: bool) {
    let option = crate::module::is_option_vec_u8(&field.ty);
    if !option && !crate::module::is_vec_u8(&field.ty) {
        return;
    }
    let with = ["with", "serialize_with", "deserialize_with"];
    if !says(&field.attrs, "serde", &with) {
        let module = if option {
            quote!(#krate::bytes::option)
        } else {
            quote!(#krate::bytes)
        };
        let path = module.to_string().replace(' ', "");
        field.attrs.push(syn::parse_quote!(#[serde(with = #path)]));
        if option && !says(&field.attrs, "serde", &["default"]) {
            field.attrs.push(syn::parse_quote!(#[serde(default)]));
        }
    }
    if types && !says(&field.attrs, "specta", &["type"]) {
        let ty = if option {
            quote!(::core::option::Option<#krate::Bytes>)
        } else {
            quote!(#krate::Bytes)
        };
        field.attrs.push(syn::parse_quote!(#[specta(type = #ty)]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_struct(out: TokenStream) -> syn::ItemStruct {
        let file: syn::File = syn::parse2(out).unwrap();
        match file.items.into_iter().next() {
            Some(Item::Struct(s)) => s,
            _ => panic!("no struct"),
        }
    }

    #[test]
    fn only_missing_derives_are_added() {
        let out = expand(
            quote!(),
            quote! {
                #[derive(Clone, serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                struct S { a_b: u8 }
            },
        );
        let s = first_struct(out);
        let derives = derived(&s.attrs);
        assert_eq!(derives.iter().filter(|d| *d == "Serialize").count(), 1);
        assert!(derives.iter().any(|d| d == "Deserialize"));
        assert_eq!(derives.iter().any(|d| d == "Type"), cfg!(feature = "types"));
        assert!(names_crate(&s.attrs, "serde"));
    }

    #[test]
    fn byte_fields_become_uint8_arrays_unless_they_say_otherwise() {
        let out = expand(
            quote!(),
            quote! {
                struct S {
                    a: Vec<u8>,
                    b: Option<Vec<u8>>,
                    #[serde(with = "mine")]
                    c: Vec<u8>,
                    d: Vec<u16>,
                }
            },
        );
        let s = first_struct(out);
        let attrs: Vec<String> = s
            .fields
            .iter()
            .map(|f| {
                let a = &f.attrs;
                quote!(#(#a)*).to_string()
            })
            .collect();
        assert!(attrs[0].contains("\"::skathi::bytes\""), "{}", attrs[0]);
        assert!(attrs[1].contains("\"::skathi::bytes::option\"") && attrs[1].contains("default"));
        assert!(!attrs[2].contains("skathi::bytes"));
        assert!(!attrs[3].contains("bytes"));
    }

    #[test]
    fn unions_and_functions_are_refused() {
        for item in [
            quote!(union U { a: u8 }),
            quote!(
                fn f() {}
            ),
        ] {
            assert!(expand(quote!(), item).to_string().contains("compile_error"));
        }
    }
}
