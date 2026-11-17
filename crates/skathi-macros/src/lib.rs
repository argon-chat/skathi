//! The attributes behind `#[skathi::module]` and `#[skathi::data]`; use them through the `skathi` crate.

use proc_macro::TokenStream;

mod data;
mod direct;
mod module;

/// Makes an `impl` block a native module: `skathi::Builder::new().module("notes", Notes { … })`.
///
/// Every `pub fn` taking `&self` is exported under its camelCase name, with its parameter names;
/// `#[task]` runs one on a worker and returns a `Promise`. Private methods and associated functions
/// stay Rust-only. `events(progress = SyncProgress)` declares what `Handle::emit_to` may send.
#[proc_macro_attribute]
pub fn module(attr: TokenStream, item: TokenStream) -> TokenStream {
    module::expand(attr.into(), item.into()).into()
}

/// Derives what a native module's argument, result or event type needs: serde's `Serialize` and
/// `Deserialize` and, with skathi's `types` feature, `skathi::Type`, all through `skathi`'s paths.
#[proc_macro_attribute]
pub fn data(attr: TokenStream, item: TokenStream) -> TokenStream {
    data::expand(attr.into(), item.into()).into()
}

fn crate_path(path: Option<syn::Path>) -> proc_macro2::TokenStream {
    match path {
        Some(path) => quote::quote!(#path),
        None => quote::quote!(::skathi),
    }
}
