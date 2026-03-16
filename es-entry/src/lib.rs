use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn module(args: TokenStream, item: TokenStream) -> TokenStream {
    let entry = parse_macro_input!(args as syn::Ident);
    let item = parse_macro_input!(item as syn::ItemFn);

    match expand_entry_mod(&entry.to_string(), item) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error().into(),
    }
}

#[proc_macro_attribute]
pub fn entry_mod(args: TokenStream, item: TokenStream) -> TokenStream {
    let entry = parse_macro_input!(args as syn::Ident);
    let item = parse_macro_input!(item as syn::ItemFn);

    match expand_entry_mod(&entry.to_string(), item) {
        Ok(tokens) => tokens,
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_entry_mod(entry: &str, mut item: syn::ItemFn) -> Result<TokenStream, syn::Error> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[module(...)] could not read CARGO_MANIFEST_DIR",
        )
    })?;
    let src_dir = PathBuf::from(manifest_dir).join("src");

    if module_source_path(&src_dir, entry).is_none() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("#[module(...)] could not find src/{entry}.rs or src/{entry}/mod.rs"),
        ));
    }

    let mut modules = BTreeSet::new();
    collect_modules(&src_dir, entry, &mut modules)?;

    let module_idents: Vec<_> = modules.iter().map(|name| format_ident!("{name}")).collect();
    let entry_ident = format_ident!("{entry}");

    item.attrs.retain(|attr| !is_arduino_hal_entry_attr(attr));

    Ok(quote! {
        #(mod #module_idents;)*
        use crate::#entry_ident as entry;
        #[arduino_hal::entry]
        #item
    }
    .into())
}

fn is_arduino_hal_entry_attr(attr: &syn::Attribute) -> bool {
    let mut segments = attr.path().segments.iter();
    matches!(
        (segments.next(), segments.next(), segments.next()),
        (Some(first), Some(second), None)
            if first.ident == "arduino_hal" && second.ident == "entry"
    )
}

fn collect_modules(
    src_dir: &Path,
    module_name: &str,
    modules: &mut BTreeSet<String>,
) -> Result<(), syn::Error> {
    if !modules.insert(module_name.to_owned()) {
        return Ok(());
    }

    let module_path = module_source_path(src_dir, module_name).ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("#[module(...)] could not resolve local module `{module_name}`"),
        )
    })?;

    let source = fs::read_to_string(&module_path).map_err(|error| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "#[module(...)] failed to read {}: {error}",
                module_path.display()
            ),
        )
    })?;
    let tokens = TokenStream2::from_str(&source).map_err(|error| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!(
                "#[module(...)] failed to tokenize {}: {error}",
                module_path.display()
            ),
        )
    })?;

    for dependency in scan_dependencies(tokens) {
        if module_source_path(src_dir, &dependency).is_some() {
            collect_modules(src_dir, &dependency, modules)?;
        }
    }

    Ok(())
}

fn module_source_path(src_dir: &Path, module_name: &str) -> Option<PathBuf> {
    let direct_file = src_dir.join(format!("{module_name}.rs"));
    if direct_file.is_file() {
        return Some(direct_file);
    }

    let nested_mod = src_dir.join(module_name).join("mod.rs");
    if nested_mod.is_file() {
        return Some(nested_mod);
    }

    None
}

fn scan_dependencies(tokens: TokenStream2) -> BTreeSet<String> {
    let mut dependencies = BTreeSet::new();
    scan_stream(tokens, &mut dependencies);
    dependencies
}

fn scan_stream(tokens: TokenStream2, dependencies: &mut BTreeSet<String>) {
    let mut iter = tokens.into_iter().peekable();

    while let Some(token) = iter.next() {
        match token {
            TokenTree::Ident(ident) if ident == "crate" => {
                if consume_double_colon(&mut iter) {
                    collect_root_dependency(&mut iter, dependencies);
                }
            }
            TokenTree::Group(group) => scan_stream(group.stream(), dependencies),
            _ => {}
        }
    }
}

fn consume_double_colon<I>(iter: &mut std::iter::Peekable<I>) -> bool
where
    I: Iterator<Item = TokenTree>,
{
    let Some(TokenTree::Punct(first)) = iter.next() else {
        return false;
    };
    if first.as_char() != ':' {
        return false;
    }

    let Some(TokenTree::Punct(second)) = iter.next() else {
        return false;
    };
    second.as_char() == ':'
}

fn collect_root_dependency<I>(
    iter: &mut std::iter::Peekable<I>,
    dependencies: &mut BTreeSet<String>,
) where
    I: Iterator<Item = TokenTree>,
{
    let Some(next_token) = iter.next() else {
        return;
    };

    match next_token {
        TokenTree::Ident(ident) => {
            dependencies.insert(ident.to_string());
        }
        TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => {
            collect_group_roots(group.stream(), dependencies);
        }
        TokenTree::Group(group) => scan_stream(group.stream(), dependencies),
        _ => {}
    }
}

fn collect_group_roots(tokens: TokenStream2, dependencies: &mut BTreeSet<String>) {
    let mut expecting_root = true;

    for token in tokens {
        match token {
            TokenTree::Ident(ident) if expecting_root => {
                let name = ident.to_string();
                if name != "self" && name != "super" && name != "crate" {
                    dependencies.insert(name);
                }
                expecting_root = false;
            }
            TokenTree::Punct(punct) if punct.as_char() == ',' => {
                expecting_root = true;
            }
            TokenTree::Group(group) => {
                scan_stream(group.stream(), dependencies);
                expecting_root = false;
            }
            _ => {
                expecting_root = false;
            }
        }
    }
}
