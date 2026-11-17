//! `FromJs`/`IntoJs` for a `#[skathi::data]` type: each field read and written through its own
//! impl, with the JS shape serde gives it. A type using a serde attribute not handled here gets
//! neither, and goes through serde.

use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::ext::IdentExt;
use syn::{Attribute, Fields, GenericParam, Generics, Ident, Item, LitStr, Path, Type};

use crate::module::{is_option_vec_u8, is_vec_u8};

#[derive(Clone, Copy, PartialEq)]
enum Case {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl Case {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "lowercase" => Case::Lower,
            "UPPERCASE" => Case::Upper,
            "PascalCase" => Case::Pascal,
            "camelCase" => Case::Camel,
            "snake_case" => Case::Snake,
            "SCREAMING_SNAKE_CASE" => Case::ScreamingSnake,
            "kebab-case" => Case::Kebab,
            "SCREAMING-KEBAB-CASE" => Case::ScreamingKebab,
            _ => return None,
        })
    }

    /// serde's rule for a field, written in snake_case.
    fn field(self, name: &str) -> String {
        match self {
            Case::Lower | Case::Snake => name.to_string(),
            Case::Upper | Case::ScreamingSnake => name.to_ascii_uppercase(),
            Case::Pascal => {
                let mut out = String::new();
                let mut capital = true;
                for c in name.chars() {
                    if c == '_' {
                        capital = true;
                    } else if capital {
                        out.extend(c.to_uppercase());
                        capital = false;
                    } else {
                        out.push(c);
                    }
                }
                out
            }
            Case::Camel => {
                let pascal = Case::Pascal.field(name);
                lower_first(&pascal)
            }
            Case::Kebab => name.replace('_', "-"),
            Case::ScreamingKebab => name.replace('_', "-").to_ascii_uppercase(),
        }
    }

    /// serde's rule for a variant, written in PascalCase.
    fn variant(self, name: &str) -> String {
        match self {
            Case::Pascal => name.to_string(),
            Case::Lower => name.to_ascii_lowercase(),
            Case::Upper => name.to_ascii_uppercase(),
            Case::Camel => lower_first(name),
            Case::Snake => {
                let mut out = String::new();
                for (i, c) in name.char_indices() {
                    if i > 0 && c.is_uppercase() {
                        out.push('_');
                    }
                    out.push(c.to_ascii_lowercase());
                }
                out
            }
            Case::ScreamingSnake => Case::Snake.variant(name).to_ascii_uppercase(),
            Case::Kebab => Case::Snake.variant(name).replace('_', "-"),
            Case::ScreamingKebab => Case::ScreamingSnake.variant(name).replace('_', "-"),
        }
    }
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The serde attributes handled here; anything else leaves the type to serde.
#[derive(Default)]
struct Serde {
    rename: Option<String>,
    rename_all: Option<Case>,
    tag: Option<String>,
    content: Option<String>,
    untagged: bool,
    skip_serializing: bool,
    skip_deserializing: bool,
    default: Option<Option<Path>>,
    skip_serializing_if: Option<Path>,
}

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Container,
    Variant,
    Field,
}

fn serde(attrs: &[Attribute], level: Level) -> Option<Serde> {
    let mut out = Serde::default();
    let mut ok = true;
    for attr in attrs.iter().filter(|a| a.path().is_ident("serde")) {
        let parsed = attr.parse_nested_meta(|meta| {
            let name = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            let string = |meta: &syn::meta::ParseNestedMeta| -> syn::Result<String> {
                Ok(meta.value()?.parse::<LitStr>()?.value())
            };
            match (name.as_str(), level) {
                ("rename", _) => out.rename = Some(string(&meta)?),
                ("rename_all", Level::Container | Level::Variant) => {
                    match Case::parse(&string(&meta)?) {
                        Some(case) => out.rename_all = Some(case),
                        None => ok = false,
                    }
                }
                ("crate", Level::Container) => {
                    string(&meta)?;
                }
                ("tag", Level::Container) => out.tag = Some(string(&meta)?),
                ("content", Level::Container) => out.content = Some(string(&meta)?),
                ("untagged", Level::Container) => out.untagged = true,
                ("skip", Level::Field) => {
                    out.skip_serializing = true;
                    out.skip_deserializing = true;
                }
                ("skip_serializing", Level::Field) => out.skip_serializing = true,
                ("skip_deserializing", Level::Field) => out.skip_deserializing = true,
                ("default", Level::Field) => {
                    out.default = Some(if meta.input.peek(syn::Token![=]) {
                        Some(meta.value()?.parse::<LitStr>()?.parse::<Path>()?)
                    } else {
                        None
                    });
                }
                ("skip_serializing_if", Level::Field) => {
                    out.skip_serializing_if = Some(meta.value()?.parse::<LitStr>()?.parse()?)
                }
                _ => {
                    ok = false;
                    if meta.input.peek(syn::Token![=]) {
                        meta.value()?.parse::<syn::Expr>()?;
                    } else if meta.input.peek(syn::token::Paren) {
                        let _ = meta.parse_nested_meta(|_| Ok(()));
                    }
                }
            }
            Ok(())
        });
        if parsed.is_err() {
            return None;
        }
    }
    ok.then_some(out)
}

#[derive(Clone, Copy)]
enum Kind {
    Plain,
    Bytes,
    OptionBytes,
}

struct Field {
    member: TokenStream,
    ty: Type,
    name: String,
    kind: Kind,
    serde: Serde,
}

fn fields(fields: &Fields, rename_all: Option<Case>) -> Option<Vec<Field>> {
    fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let serde = serde(&f.attrs, Level::Field)?;
            let member = match &f.ident {
                Some(ident) => quote!(#ident),
                None => {
                    let index = syn::Index::from(i);
                    quote!(#index)
                }
            };
            let own = f
                .ident
                .as_ref()
                .map(|i| i.unraw().to_string())
                .unwrap_or_default();
            let name = match (&serde.rename, rename_all) {
                (Some(name), _) => name.clone(),
                (None, Some(case)) => case.field(&own),
                (None, None) => own,
            };
            let kind = if is_vec_u8(&f.ty) {
                Kind::Bytes
            } else if is_option_vec_u8(&f.ty) {
                Kind::OptionBytes
            } else {
                Kind::Plain
            };
            Some(Field {
                member,
                ty: f.ty.clone(),
                name,
                kind,
                serde,
            })
        })
        .collect()
}

struct Variant {
    ident: Ident,
    name: String,
    shape: Shape,
}

enum Shape {
    Unit,
    Newtype(Box<Field>),
    Tuple(Vec<Field>),
    Struct(Vec<Field>),
}

enum Tagging {
    External,
    Internal(String),
    Adjacent(String, String),
    Untagged,
}

pub(crate) fn expand(item: &Item, krate: &TokenStream) -> TokenStream {
    let (attrs, ident, generics) = match item {
        Item::Struct(s) => (&s.attrs, &s.ident, &s.generics),
        Item::Enum(e) => (&e.attrs, &e.ident, &e.generics),
        _ => return TokenStream::new(),
    };
    let Some(container) = serde(attrs, Level::Container) else {
        return TokenStream::new();
    };
    match item {
        Item::Struct(s) => {
            if container.tag.is_some() || container.content.is_some() || container.untagged {
                return TokenStream::new();
            }
            let Some(fields) = fields(&s.fields, container.rename_all) else {
                return TokenStream::new();
            };
            match &s.fields {
                Fields::Named(_) => named_struct(ident, generics, &fields, krate),
                Fields::Unnamed(_) if fields.len() == 1 && fields[0].serde.is_plain() => {
                    newtype_struct(ident, generics, &fields[0], krate)
                }
                _ => TokenStream::new(),
            }
        }
        Item::Enum(e) => {
            let tagging = match (&container.tag, &container.content, container.untagged) {
                (None, None, false) => Tagging::External,
                (Some(tag), None, false) => Tagging::Internal(tag.clone()),
                (Some(tag), Some(content), false) => {
                    Tagging::Adjacent(tag.clone(), content.clone())
                }
                (None, None, true) => Tagging::Untagged,
                _ => return TokenStream::new(),
            };
            let mut variants = Vec::new();
            for v in &e.variants {
                let Some(own) = serde(&v.attrs, Level::Variant) else {
                    return TokenStream::new();
                };
                let name = match (&own.rename, container.rename_all) {
                    (Some(name), _) => name.clone(),
                    (None, Some(case)) => case.variant(&v.ident.unraw().to_string()),
                    (None, None) => v.ident.unraw().to_string(),
                };
                let Some(mut fields) = fields(&v.fields, own.rename_all) else {
                    return TokenStream::new();
                };
                let shape = match &v.fields {
                    Fields::Unit => Shape::Unit,
                    Fields::Named(_) => Shape::Struct(fields),
                    Fields::Unnamed(_) => {
                        if fields.iter().any(|f| !f.serde.is_plain()) {
                            return TokenStream::new();
                        }
                        if fields.len() == 1 {
                            Shape::Newtype(Box::new(fields.remove(0)))
                        } else {
                            Shape::Tuple(fields)
                        }
                    }
                };
                variants.push(Variant {
                    ident: v.ident.clone(),
                    name,
                    shape,
                });
            }
            enumeration(ident, generics, &variants, &tagging, krate)
        }
        _ => TokenStream::new(),
    }
}

impl Serde {
    fn is_plain(&self) -> bool {
        self.rename.is_none()
            && !self.skip_serializing
            && !self.skip_deserializing
            && self.default.is_none()
            && self.skip_serializing_if.is_none()
    }
}

/// The impl header's pieces, with `bound` on every type parameter and `Self: also`.
fn header(
    generics: &Generics,
    bound: &TokenStream,
    also: &TokenStream,
) -> (TokenStream, TokenStream, TokenStream) {
    let (impl_g, ty_g, where_clause) = generics.split_for_impl();
    let mut predicates: Vec<TokenStream> = where_clause
        .map(|w| w.predicates.iter().map(|p| p.to_token_stream()).collect())
        .unwrap_or_default();
    for param in &generics.params {
        if let GenericParam::Type(t) = param {
            let ident = &t.ident;
            predicates.push(quote!(#ident: #bound));
        }
    }
    predicates.push(quote!(Self: #also));
    (
        impl_g.to_token_stream(),
        ty_g.to_token_stream(),
        quote!(where #(#predicates),*),
    )
}

fn has_lifetimes(generics: &Generics) -> bool {
    generics.lifetimes().next().is_some()
}

fn private(krate: &TokenStream) -> TokenStream {
    quote!(#krate::__private)
}

/// Reads field `index` of `fields` into the field's type.
fn read_field(f: &Field, index: usize, p: &TokenStream, krate: &TokenStream) -> TokenStream {
    let ty = &f.ty;
    let got = match f.kind {
        Kind::Plain => {
            quote!(fields.get(#index, |js| (&&#p::Pick::<#ty>::new()).read(js))?)
        }
        Kind::Bytes => quote! {
            fields.get(#index, |js| (&&#p::Pick::<#krate::Bytes>::new()).read(js))?.map(|b| b.0)
        },
        Kind::OptionBytes => quote! {
            fields
                .get(#index, |js| (&&#p::Pick::<::core::option::Option<#krate::Bytes>>::new()).read(js))?
                .map(|b| b.map(|b| b.0))
        },
    };
    let missing = missing(f, p);
    quote! {
        match #got {
            ::core::option::Option::Some(v) => v,
            ::core::option::Option::None => #missing,
        }
    }
}

fn missing(f: &Field, p: &TokenStream) -> TokenStream {
    let ty = &f.ty;
    let name = &f.name;
    match (&f.serde.default, f.kind) {
        (Some(Some(path)), _) => quote!(#path()),
        (Some(None), _) | (None, Kind::OptionBytes) => quote!(::core::default::Default::default()),
        (None, _) => quote!(#p::missing::<#ty>(#name)?),
    }
}

/// The initializers of a struct's fields, read from an object; the names it reads.
fn struct_body(
    fields: &[Field],
    p: &TokenStream,
    krate: &TokenStream,
) -> (Vec<String>, Vec<TokenStream>) {
    let mut names = Vec::new();
    let mut inits = Vec::new();
    for f in fields {
        let member = &f.member;
        if f.serde.skip_deserializing {
            let value = match &f.serde.default {
                Some(Some(path)) => quote!(#path()),
                _ => quote!(::core::default::Default::default()),
            };
            inits.push(quote!(#member: #value));
            continue;
        }
        let read = read_field(f, names.len(), p, krate);
        names.push(f.name.clone());
        inits.push(quote!(#member: #read));
    }
    (names, inits)
}

/// Writes `value` (an expression of the field's type, by reference) into `out`.
fn write_value(f: &Field, value: TokenStream, p: &TokenStream) -> TokenStream {
    let ty = &f.ty;
    match f.kind {
        Kind::Plain => quote!((&&#p::Pick::<#ty>::new()).encode(#value, out)?;),
        Kind::Bytes => {
            quote!(#p::encode_bytes(::core::option::Option::Some(::core::convert::AsRef::<[u8]>::as_ref(#value)), out);)
        }
        Kind::OptionBytes => quote!(#p::encode_bytes((#value).as_deref(), out);),
    }
}

/// Writes the fields (`access(member)` gives each one, by reference) as an object's entries,
/// adding to `count`.
fn write_fields(
    fields: &[Field],
    access: impl Fn(&Field) -> TokenStream,
    p: &TokenStream,
) -> Vec<TokenStream> {
    fields
        .iter()
        .filter(|f| !f.serde.skip_serializing)
        .map(|f| {
            let name = &f.name;
            let value = access(f);
            let write = write_value(f, value.clone(), p);
            let entry = quote! {
                out.key(#name);
                #write
                count += 1;
            };
            match &f.serde.skip_serializing_if {
                Some(path) => quote!(if !#path(#value) { #entry }),
                None => entry,
            }
        })
        .collect()
}

fn into_js(
    ident: &Ident,
    generics: &Generics,
    encode: TokenStream,
    compound: bool,
    krate: &TokenStream,
) -> TokenStream {
    let p = private(krate);
    let (impl_g, ty_g, where_clause) = header(
        generics,
        &quote!(#p::IntoJs),
        &quote!(#krate::serde::Serialize),
    );
    let answers = compound.then(|| {
        quote! {
            fn answer(&self, context: &#p::Context) -> ::core::result::Result<#p::Value, #p::OutError> {
                #p::answer_compound(self, context)
            }
            fn answer_owned(&self) -> ::core::result::Result<#p::Owned, #p::OutError> {
                #p::owned_compound(self)
            }
        }
    });
    quote! {
        #[automatically_derived]
        impl #impl_g #p::IntoJs for #ident #ty_g #where_clause {
            fn encode(&self, out: #p::Tree<'_>) -> ::core::result::Result<(), #p::OutError> {
                #[allow(unused_imports)]
                use #p::{WriteDirect as _, WriteSerde as _};
                #encode
                ::core::result::Result::Ok(())
            }
            #answers
        }
    }
}

fn from_js(
    ident: &Ident,
    generics: &Generics,
    body: TokenStream,
    krate: &TokenStream,
) -> TokenStream {
    if has_lifetimes(generics) {
        return TokenStream::new();
    }
    let p = private(krate);
    let (impl_g, ty_g, where_clause) = header(
        generics,
        &quote!(#p::FromJs),
        &quote!(#krate::serde::de::DeserializeOwned),
    );
    quote! {
        #[automatically_derived]
        impl #impl_g #p::FromJs for #ident #ty_g #where_clause {
            fn from_js(js: #p::Js<'_, '_>) -> ::core::result::Result<Self, #p::Error> {
                #[allow(unused_imports)]
                use #p::{ReadDirect as _, ReadSerde as _};
                #body
            }
        }
    }
}

fn named_struct(
    ident: &Ident,
    generics: &Generics,
    fields: &[Field],
    krate: &TokenStream,
) -> TokenStream {
    let p = private(krate);
    let (names, inits) = struct_body(fields, &p, krate);
    let count = names.len();
    let read = quote! {
        static KEYS: #p::Keys = #p::Keys::new(&[#(#names),*]);
        match #p::read_struct::<_, #count>(&js, &KEYS, |fields| {
            ::core::result::Result::Ok(Self { #(#inits),* })
        })? {
            ::core::option::Option::Some(v) => ::core::result::Result::Ok(v),
            ::core::option::Option::None => <Self as #krate::serde::Deserialize>::deserialize(js),
        }
    };
    let writes = write_fields(
        fields,
        |f| {
            let member = &f.member;
            quote!(&self.#member)
        },
        &p,
    );
    let encode = quote! {
        let open = out.begin_object();
        let mut count = 0u32;
        #(#writes)*
        out.end(open, count);
    };
    let from = from_js(ident, generics, read, krate);
    let into = into_js(ident, generics, encode, true, krate);
    quote!(#from #into)
}

fn newtype_struct(
    ident: &Ident,
    generics: &Generics,
    field: &Field,
    krate: &TokenStream,
) -> TokenStream {
    let p = private(krate);
    let ty = &field.ty;
    let read = match field.kind {
        Kind::Plain => quote!((&&#p::Pick::<#ty>::new()).read(js).map(Self)),
        _ => quote!(<Self as #krate::serde::Deserialize>::deserialize(js)),
    };
    let write = write_value(field, quote!(&self.0), &p);
    let answers = matches!(field.kind, Kind::Plain).then(|| {
        quote! {
            fn answer(&self, context: &#p::Context) -> ::core::result::Result<#p::Value, #p::OutError> {
                #[allow(unused_imports)]
                use #p::{WriteDirect as _, WriteSerde as _};
                (&&#p::Pick::<#ty>::new()).answer(&self.0, context)
            }
            fn answer_owned(&self) -> ::core::result::Result<#p::Owned, #p::OutError> {
                #[allow(unused_imports)]
                use #p::{WriteDirect as _, WriteSerde as _};
                (&&#p::Pick::<#ty>::new()).answer_owned(&self.0)
            }
        }
    });
    let from = from_js(ident, generics, read, krate);
    let (impl_g, ty_g, where_clause) = header(
        generics,
        &quote!(#p::IntoJs),
        &quote!(#krate::serde::Serialize),
    );
    quote! {
        #from
        #[automatically_derived]
        impl #impl_g #p::IntoJs for #ident #ty_g #where_clause {
            fn encode(&self, out: #p::Tree<'_>) -> ::core::result::Result<(), #p::OutError> {
                #[allow(unused_imports)]
                use #p::{WriteDirect as _, WriteSerde as _};
                #write
                ::core::result::Result::Ok(())
            }
            #answers
        }
    }
}

fn enumeration(
    ident: &Ident,
    generics: &Generics,
    variants: &[Variant],
    tagging: &Tagging,
    krate: &TokenStream,
) -> TokenStream {
    let p = private(krate);
    let mut arms = Vec::new();
    for v in variants {
        let vi = &v.ident;
        let name = &v.name;
        let arm = match (&v.shape, tagging) {
            (Shape::Unit, Tagging::External) => quote!(Self::#vi => out.str_value(#name),),
            (Shape::Unit, Tagging::Internal(tag) | Tagging::Adjacent(tag, _)) => quote! {
                Self::#vi => {
                    let open = out.begin_object();
                    out.key(#tag);
                    out.str_value(#name);
                    out.end(open, 1);
                }
            },
            (Shape::Unit, Tagging::Untagged) => quote!(Self::#vi => out.null_value(),),
            (Shape::Newtype(_) | Shape::Tuple(_), Tagging::Internal(_)) => {
                quote!(Self::#vi(..) => #p::encode_serde(self, out)?,)
            }
            (Shape::Newtype(f), _) => {
                let write = write_value(f, quote!(v), &p);
                wrap(vi, quote!((v)), write, name, tagging)
            }
            (Shape::Tuple(fields), _) => {
                let binds: Vec<_> = (0..fields.len())
                    .map(|i| Ident::new(&format!("v{i}"), Span::mixed_site()))
                    .collect();
                let writes = fields
                    .iter()
                    .zip(&binds)
                    .map(|(f, b)| write_value(f, quote!(#b), &p));
                let n = fields.len() as u32;
                let write = quote! {
                    let open = out.begin_array();
                    #(#writes)*
                    out.end(open, #n);
                };
                wrap(vi, quote!((#(#binds),*)), write, name, tagging)
            }
            (Shape::Struct(fields), _) => {
                let binds: Vec<_> = (0..fields.len())
                    .map(|i| Ident::new(&format!("v{i}"), Span::mixed_site()))
                    .collect();
                let members = fields.iter().map(|f| &f.member);
                let pattern = quote!({ #(#members: #binds),* });
                let by_member: Vec<_> = fields
                    .iter()
                    .zip(&binds)
                    .map(|(f, b)| (f.member.to_string(), b.clone()))
                    .collect();
                let access = |f: &Field| {
                    let m = f.member.to_string();
                    let b = &by_member.iter().find(|(n, _)| *n == m).unwrap().1;
                    quote!(#b)
                };
                let writes = write_fields(fields, access, &p);
                match tagging {
                    Tagging::Internal(tag) => quote! {
                        #[allow(unused_variables)]
                        Self::#vi #pattern => {
                            let open = out.begin_object();
                            let mut count = 1u32;
                            out.key(#tag);
                            out.str_value(#name);
                            #(#writes)*
                            out.end(open, count);
                        }
                    },
                    _ => {
                        let write = quote! {
                            let open = out.begin_object();
                            let mut count = 0u32;
                            #(#writes)*
                            out.end(open, count);
                        };
                        let arm = wrap(vi, pattern, write, name, tagging);
                        quote!(#[allow(unused_variables)] #arm)
                    }
                }
            }
        };
        arms.push(arm);
    }
    let encode = if variants.is_empty() {
        quote!(match *self {})
    } else {
        quote!(match self { #(#arms)* })
    };
    let into = if ident == "Result" {
        TokenStream::new()
    } else {
        into_js(ident, generics, encode, true, krate)
    };

    let units: Vec<_> = variants
        .iter()
        .filter(|v| matches!(v.shape, Shape::Unit))
        .map(|v| {
            let vi = &v.ident;
            let name = &v.name;
            quote!(#name => ::core::option::Option::Some(Self::#vi),)
        })
        .collect();
    let from = if matches!(tagging, Tagging::External) && !units.is_empty() {
        from_js(
            ident,
            generics,
            quote! {
                match #p::unit_variant(&js, |name| match name {
                    #(#units)*
                    _ => ::core::option::Option::None,
                })? {
                    ::core::option::Option::Some(v) => ::core::result::Result::Ok(v),
                    ::core::option::Option::None => <Self as #krate::serde::Deserialize>::deserialize(js),
                }
            },
            krate,
        )
    } else {
        from_js(
            ident,
            generics,
            quote!(<Self as #krate::serde::Deserialize>::deserialize(js)),
            krate,
        )
    };
    quote!(#from #into)
}

/// The arm for a variant with data: `write` writes the data, tagged as `tagging` says.
fn wrap(
    vi: &Ident,
    pattern: TokenStream,
    write: TokenStream,
    name: &str,
    tagging: &Tagging,
) -> TokenStream {
    match tagging {
        Tagging::External => quote! {
            Self::#vi #pattern => {
                let open = out.begin_object();
                out.key(#name);
                { #write }
                out.end(open, 1);
            }
        },
        Tagging::Adjacent(tag, content) => quote! {
            Self::#vi #pattern => {
                let open = out.begin_object();
                out.key(#tag);
                out.str_value(#name);
                out.key(#content);
                { #write }
                out.end(open, 2);
            }
        },
        Tagging::Untagged => quote!(Self::#vi #pattern => { #write }),
        Tagging::Internal(_) => unreachable!(),
    }
}
