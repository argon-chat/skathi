use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{FnArg, ImplItem, ItemImpl, LitStr, Pat, ReturnType, Token, Type, Visibility};

const MAX_PARAMS: usize = 8;

/// JS keywords and the names every module already exports.
const RESERVED: &str = "await break case catch class const continue debugger delete do else enum \
    export extends false finally for function if implements import in instanceof interface let new \
    null package private protected public return static super switch this throw true try typeof var \
    void while with yield available on off default then __esModule";

struct EventDecl {
    name: LitStr,
    ty: Type,
}

impl Parse for EventDecl {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name = if input.peek(LitStr) {
            input.parse::<LitStr>()?
        } else {
            let ident: Ident = input.call(syn::ext::IdentExt::parse_any)?;
            LitStr::new(&ident.to_string(), ident.span())
        };
        input.parse::<Token![=]>()?;
        Ok(Self {
            name,
            ty: input.parse()?,
        })
    }
}

#[derive(Default)]
struct Args {
    krate: Option<syn::Path>,
    events: Vec<EventDecl>,
}

fn parse_args(attr: TokenStream) -> syn::Result<Args> {
    let mut args = Args::default();
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("crate") {
            args.krate = Some(meta.value()?.parse()?);
            Ok(())
        } else if meta.path.is_ident("events") {
            let content;
            syn::parenthesized!(content in meta.input);
            let events = Punctuated::<EventDecl, Token![,]>::parse_terminated(&content)?;
            args.events.extend(events);
            Ok(())
        } else {
            Err(meta.error("expected `events(name = Type, …)` or `crate = path`"))
        }
    });
    syn::parse::Parser::parse2(parser, attr)?;
    Ok(args)
}

/// `fetch_all` → `fetchAll`; leading underscores stay.
pub(crate) fn camel_case(name: &str) -> String {
    let name = name.strip_prefix("r#").unwrap_or(name);
    let body = name.trim_start_matches('_');
    let mut out = name[..name.len() - body.len()].to_owned();
    let mut upper = false;
    for c in body.chars() {
        if c == '_' {
            upper = !out.is_empty() && out.chars().any(|c| c != '_');
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn is_task(attr: &syn::Attribute) -> bool {
    attr.path().is_ident("task")
}

struct Export {
    js: String,
    method: syn::Ident,
    task: bool,
    params: Vec<(String, Param)>,
    ret: Bytes,
}

enum Param {
    Owned(Type),
    Bytes(Bytes),
    Borrowed(Borrowed),
}

/// Where a `Vec<u8>` sits in a parameter or a result: it goes to JS as a `Uint8Array`.
#[derive(Clone, Copy, PartialEq)]
enum Bytes {
    None,
    Plain,
    Option,
    Result,
}

const ELEMENTS: [&str; 10] = [
    "u8", "i8", "i16", "u16", "i32", "u32", "f32", "f64", "i64", "u64",
];

/// The first type argument of `Name<T, …>`.
fn argument_of<'t>(ty: &'t Type, name: &str) -> Option<&'t Type> {
    let Type::Path(p) = ty else { return None };
    let last = p.path.segments.last()?;
    if p.qself.is_some() || last.ident != name {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &last.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(t) => Some(t),
        _ => None,
    }
}

fn is_ident(ty: &Type, name: &str) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident(name))
}

pub(crate) fn is_vec_u8(ty: &Type) -> bool {
    argument_of(ty, "Vec").is_some_and(|t| is_ident(t, "u8"))
}

pub(crate) fn is_option_vec_u8(ty: &Type) -> bool {
    argument_of(ty, "Option").is_some_and(is_vec_u8)
}

impl Bytes {
    fn of(ty: &Type, result: bool) -> Self {
        if is_vec_u8(ty) {
            Bytes::Plain
        } else if is_option_vec_u8(ty) {
            Bytes::Option
        } else if result && argument_of(ty, "Result").is_some_and(is_vec_u8) {
            Bytes::Result
        } else {
            Bytes::None
        }
    }
}

/// A parameter read in place for the length of a synchronous call.
enum Borrowed {
    Str,
    Slice(Type),
}

impl Borrowed {
    fn of(r: &syn::TypeReference) -> Option<Self> {
        if r.mutability.is_some() || r.lifetime.as_ref().is_some_and(|l| l.ident != "_") {
            return None;
        }
        match &*r.elem {
            t if is_ident(t, "str") => Some(Borrowed::Str),
            Type::Slice(s) if ELEMENTS.iter().any(|e| is_ident(&s.elem, e)) => {
                Some(Borrowed::Slice((*s.elem).clone()))
            }
            _ => None,
        }
    }

    fn owned(&self) -> String {
        match self {
            Borrowed::Str => "`String`".into(),
            Borrowed::Slice(t) if is_ident(t, "u8") => "`Vec<u8>` or `skathi::Bytes`".into(),
            Borrowed::Slice(t) => format!("`Vec<{}>`", quote!(#t)),
        }
    }
}

fn export(method: &syn::ImplItemFn, task: bool) -> syn::Result<Option<Export>> {
    let sig = &method.sig;
    let public = matches!(method.vis, Visibility::Public(_));
    let Some(receiver) = sig.receiver() else {
        if task {
            return Err(syn::Error::new_spanned(
                &sig.ident,
                "#[task] needs a method taking &self",
            ));
        }
        return Ok(None);
    };
    if !public {
        if task {
            return Err(syn::Error::new_spanned(
                &sig.ident,
                "only `pub` methods are exported: make it `pub fn` or drop #[task]",
            ));
        }
        return Ok(None);
    }
    if receiver.reference.is_none()
        || receiver.mutability.is_some()
        || receiver.colon_token.is_some()
    {
        return Err(syn::Error::new_spanned(
            receiver,
            "an exported method takes `&self`: the module is shared by the UI thread and the task \
             workers, so keep mutable state behind a Mutex (or make the method private)",
        ));
    }
    if let Some(asyncness) = &sig.asyncness {
        return Err(syn::Error::new_spanned(
            asyncness,
            "an exported method cannot be `async`: mark it #[task] to run it on a worker thread",
        ));
    }
    if let Some(unsafety) = &sig.unsafety {
        return Err(syn::Error::new_spanned(
            unsafety,
            "an exported method cannot be `unsafe`",
        ));
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &sig.generics,
            "an exported method cannot be generic: the app calls it with concrete types",
        ));
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        if let Type::ImplTrait(_) = **ty {
            return Err(syn::Error::new_spanned(
                ty,
                "return a concrete type, not `impl Trait`",
            ));
        }
    }
    let mut params = Vec::new();
    for input in sig.inputs.iter().skip(1) {
        let FnArg::Typed(arg) = input else { continue };
        let Pat::Ident(pat) = &*arg.pat else {
            return Err(syn::Error::new_spanned(
                &arg.pat,
                "name the parameter: its name is the parameter's name in TypeScript",
            ));
        };
        if pat.by_ref.is_some() || pat.subpat.is_some() {
            return Err(syn::Error::new_spanned(
                pat,
                "name the parameter with a plain identifier",
            ));
        }
        match &*arg.ty {
            Type::Reference(r) => {
                let Some(borrowed) = Borrowed::of(r) else {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        "take an owned type (`String`, not `&String`): only `&str` and slices of \
                         numbers (`&[u8]`, `&[f32]`, …) are read in place",
                    ));
                };
                if task {
                    return Err(syn::Error::new_spanned(
                        &arg.ty,
                        format!(
                            "a #[task] runs on a worker after the call has returned, so it cannot \
                             borrow from it: take {} instead",
                            borrowed.owned()
                        ),
                    ));
                }
                params.push((
                    camel_case(&pat.ident.to_string()),
                    Param::Borrowed(borrowed),
                ));
                continue;
            }
            Type::ImplTrait(_) => {
                return Err(syn::Error::new_spanned(
                    &arg.ty,
                    "take a concrete type, not `impl Trait`",
                ))
            }
            _ => {}
        }
        let param = match Bytes::of(&arg.ty, false) {
            Bytes::None => Param::Owned((*arg.ty).clone()),
            bytes => Param::Bytes(bytes),
        };
        params.push((camel_case(&pat.ident.to_string()), param));
    }
    if params.len() > MAX_PARAMS {
        return Err(syn::Error::new_spanned(
            &sig.inputs,
            format!(
                "an exported method takes at most {MAX_PARAMS} parameters: group them in a struct"
            ),
        ));
    }
    let js = camel_case(&sig.ident.to_string());
    if RESERVED.split_whitespace().any(|r| r == js) {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            format!("`{js}` cannot be a module export: it is a JS keyword or one of available, on, off, default, then"),
        ));
    }
    Ok(Some(Export {
        js,
        method: sig.ident.clone(),
        task,
        params,
        ret: match &sig.output {
            ReturnType::Type(_, ty) => Bytes::of(ty, true),
            ReturnType::Default => Bytes::None,
        },
    }))
}

pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut block = match syn::parse2::<ItemImpl>(item.clone()) {
        Ok(block) => block,
        Err(_) => {
            let error = syn::Error::new(
                Span::call_site(),
                "#[skathi::module] goes on an inherent `impl` block: `#[skathi::module] impl Notes { … }`",
            );
            let error = error.to_compile_error();
            return quote!(#error #item);
        }
    };
    let mut errors: Vec<syn::Error> = Vec::new();
    let args = parse_args(attr).unwrap_or_else(|e| {
        errors.push(e);
        Args::default()
    });

    if let Some((_, path, _)) = &block.trait_ {
        errors.push(syn::Error::new_spanned(
            path,
            "#[skathi::module] goes on an inherent `impl` block, not a trait impl",
        ));
    }
    if !block.generics.params.is_empty() || block.generics.where_clause.is_some() {
        errors.push(syn::Error::new_spanned(
            &block.generics,
            "a native module type cannot be generic",
        ));
    }
    if block.unsafety.is_some() {
        errors.push(syn::Error::new_spanned(
            block.unsafety,
            "a native module cannot be an `unsafe impl`",
        ));
    }

    let mut exports = Vec::new();
    for item in &mut block.items {
        let ImplItem::Fn(method) = item else { continue };
        let task = method.attrs.iter().any(is_task);
        method.attrs.retain(|a| !is_task(a));
        match export(method, task) {
            Ok(Some(e)) => exports.push(e),
            Ok(None) => {}
            Err(e) => errors.push(e),
        }
    }

    if let Some(first) = errors.into_iter().reduce(|mut a, b| {
        a.combine(b);
        a
    }) {
        let error = first.to_compile_error();
        return quote!(#error #block);
    }

    let krate = crate::crate_path(args.krate);
    let ty = &block.self_ty;
    let this = Ident::new("this", Span::mixed_site());
    let module = Ident::new("module", Span::mixed_site());
    let registrations = exports.iter().map(|e| {
        let Export {
            js,
            method,
            task,
            params,
            ret,
        } = e;
        let names = params.iter().map(|(name, _)| name);
        let args: Vec<_> = (0..params.len())
            .map(|i| format_ident!("arg{}", i, span = Span::mixed_site()))
            .collect();
        let types: Vec<_> = params
            .iter()
            .map(|(_, param)| match param {
                Param::Owned(ty) => quote!(#ty),
                Param::Bytes(Bytes::Option) => quote!(::core::option::Option<#krate::Bytes>),
                Param::Bytes(_) => quote!(#krate::Bytes),
                Param::Borrowed(Borrowed::Str) => quote!(#krate::__private::Str),
                Param::Borrowed(Borrowed::Slice(t)) => quote!(#krate::__private::Slice<#t>),
            })
            .collect();
        let values: Vec<_> = params
            .iter()
            .zip(&args)
            .map(|((_, param), arg)| match param {
                Param::Owned(_) => quote!(#arg),
                Param::Bytes(Bytes::Option) => quote!(#arg.map(|b| b.0)),
                Param::Bytes(_) => quote!(#arg.0),
                // SAFETY: read in this call, used only inside it.
                Param::Borrowed(_) => quote!(unsafe { #arg.get() }),
            })
            .collect();
        let positions = (1..=params.len() as u32).map(|p| quote!(#p));
        let names = (!params.is_empty()).then(|| quote!(.params([#(#names),*])));
        let call = quote!(<#ty>::#method(&*#this, #(#values),*));
        let call = match ret {
            Bytes::None => call,
            Bytes::Plain => quote!(#krate::Bytes(#call)),
            Bytes::Option | Bytes::Result => quote!(#call.map(#krate::Bytes)),
        };
        let cx = Ident::new("cx", Span::mixed_site());
        let argv = Ident::new("argv", Span::mixed_site());
        let out = Ident::new("out", Span::mixed_site());
        let signature = Ident::new("signature", Span::mixed_site());
        let read = quote! {
            #[allow(unused_imports)]
            use #krate::__private::{ReadDirect as _, ReadSerde as _, WriteDirect as _, WriteSerde as _};
            #(let #args: #types = #krate::__private::argument(#cx, #argv, #positions, |js| {
                (&&#krate::__private::Pick::<#types>::new()).read(js)
            })?;)*
        };
        let body = if *task {
            quote! {
                #module.__task(#js, #signature, move |#cx: &#krate::__private::Context, #argv: &#krate::__private::HostArgs| {
                    #read
                    let #this = ::std::sync::Arc::clone(&#this);
                    ::core::result::Result::Ok(#krate::__private::work(move || {
                        let #out = #call;
                        (&&#krate::__private::Pick::of(&#out)).answer_owned(&#out)
                    }))
                })
            }
        } else {
            quote! {
                #module.__function(#js, #signature, move |#cx: &#krate::__private::Context, #argv: &#krate::__private::HostArgs| {
                    #read
                    let #out = #call;
                    (&&#krate::__private::Pick::of(&#out))
                        .answer(&#out, #cx)
                        .map_err(#krate::__private::fail)
                })
            }
        };
        quote! {
            {
                let #this = ::std::sync::Arc::clone(&#this);
                let #signature = {
                    let #this = ::std::sync::Arc::clone(&#this);
                    move |#(#args: #types),*| #call
                };
                #body #names;
            }
        }
    });
    let events = args
        .events
        .iter()
        .map(|EventDecl { name, ty }| quote_spanned!(ty.span()=> #module.event::<#ty>(#name);));
    let shared = quote_spanned! {ty.span()=>
        fn shared<T: ::core::marker::Send + ::core::marker::Sync + 'static>() {}
        shared::<#ty>();
    };

    quote! {
        #block

        impl #krate::IntoModule for #ty {
            fn register(self, #module: &mut #krate::Module) {
                #shared
                let #this = ::std::sync::Arc::new(self);
                #(#registrations)*
                #(#events)*
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fails(attr: TokenStream, item: TokenStream) -> bool {
        expand(attr, item).to_string().contains("compile_error")
    }

    #[test]
    fn names_become_camel_case() {
        assert_eq!(camel_case("fetch_all"), "fetchAll");
        assert_eq!(camel_case("get"), "get");
        assert_eq!(camel_case("_private_thing"), "_privateThing");
        assert_eq!(camel_case("r#type"), "type");
        assert_eq!(camel_case("to_2d"), "to2d");
    }

    #[test]
    fn unsupported_signatures_are_compile_errors() {
        let bad = [
            quote!(impl N { pub fn f(&mut self) {} }),
            quote!(impl N { pub fn f(self) {} }),
            quote!(impl N { pub fn f(self: std::sync::Arc<Self>) {} }),
            quote!(impl N { pub async fn f(&self) {} }),
            quote!(impl N { pub fn f<T>(&self, t: T) {} }),
            quote!(impl N { pub fn f(&self, (a, b): (u8, u8)) {} }),
            quote!(impl N { pub fn f(&self, s: &String) {} }),
            quote!(impl N { pub fn f(&self, s: &'static str) {} }),
            quote!(impl N { #[task] pub fn f(&self, s: &str) {} }),
            quote!(impl N { #[task] pub fn f(&self, b: &[u8]) {} }),
            quote!(impl N { pub fn f(&self, b: &mut [u8]) {} }),
            quote!(impl N { pub fn f(&self, b: &[bool]) {} }),
            quote!(impl N { pub fn f(&self, a: u8, b: u8, c: u8, d: u8, e: u8, f: u8, g: u8, h: u8, i: u8) {} }),
            quote!(impl N { pub fn delete(&self) {} }),
            quote!(impl N { #[task] fn f(&self) {} }),
            quote!(impl N { #[task] pub fn f() {} }),
            quote!(
                impl<T> N<T> {
                    pub fn f(&self) {}
                }
            ),
            quote!(impl Clone for N { fn clone(&self) -> Self { N } }),
            quote!(
                struct N;
            ),
        ];
        for item in bad {
            assert!(fails(quote!(), item.clone()), "accepted: {item}");
        }
        assert!(fails(quote!(nope), quote!(impl N {})));
    }
}
