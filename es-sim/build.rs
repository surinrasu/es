use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};

fn main() {
    let sim_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));
    let project_dir = sim_dir
        .parent()
        .expect("es-sim should live under the project root")
        .to_path_buf();
    let src_dir = project_dir.join("src");
    let main_path = src_dir.join("main.rs");

    println!("cargo:rerun-if-changed={}", main_path.display());

    let active_entry = active_entry_name(&main_path);
    let mut modules = BTreeSet::new();
    collect_modules(&src_dir, &active_entry, &mut modules);

    let generated = generate_modules(&src_dir, &active_entry, &modules);
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let out_path = out_dir.join("generated_sim_tests.rs");

    fs::write(out_path, generated.to_string()).expect("failed to write generated sim tests");
}

fn active_entry_name(main_path: &Path) -> String {
    let source = fs::read_to_string(main_path).expect("failed to read src/main.rs");
    let file = syn::parse_file(&source).expect("failed to parse src/main.rs");

    for item in file.items {
        let syn::Item::Fn(item_fn) = item else {
            continue;
        };

        for attr in item_fn.attrs {
            if !is_entry_attr(&attr) {
                continue;
            }

            return attr
                .parse_args::<syn::Ident>()
                .expect("failed to parse #[es_entry::module(...)]")
                .to_string();
        }
    }

    panic!("could not find #[es_entry::module(...)] in src/main.rs");
}

fn is_entry_attr(attr: &syn::Attribute) -> bool {
    let segments: Vec<_> = attr
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    matches!(
        segments.as_slice(),
        [entry, name] if entry == "es_entry" && (name == "module" || name == "entry_mod")
    )
}

fn collect_modules(src_dir: &Path, module_name: &str, modules: &mut BTreeSet<String>) {
    if !modules.insert(module_name.to_owned()) {
        return;
    }

    let module_path = module_source_path(src_dir, module_name)
        .unwrap_or_else(|| panic!("failed to resolve module `{module_name}`"));
    println!("cargo:rerun-if-changed={}", module_path.display());

    let source =
        fs::read_to_string(&module_path).unwrap_or_else(|err| panic!("failed to read {module_name}: {err}"));
    let tokens = TokenStream2::from_str(&source)
        .unwrap_or_else(|err| panic!("failed to tokenize {}: {err}", module_path.display()));

    for dependency in scan_dependencies(tokens) {
        if module_source_path(src_dir, &dependency).is_some() {
            collect_modules(src_dir, &dependency, modules);
        }
    }
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

fn collect_root_dependency<I>(iter: &mut std::iter::Peekable<I>, dependencies: &mut BTreeSet<String>)
where
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

fn generate_modules(src_dir: &Path, active_entry: &str, modules: &BTreeSet<String>) -> TokenStream2 {
    let mut generated = Vec::new();

    for module_name in modules {
        let module_path = module_source_path(src_dir, module_name)
            .unwrap_or_else(|| panic!("failed to resolve module `{module_name}`"));
        let source =
            fs::read_to_string(&module_path).unwrap_or_else(|err| panic!("failed to read {module_name}: {err}"));
        let file = syn::parse_file(&source)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", module_path.display()));

        if let Some(tokens) = generate_module(module_name, active_entry == module_name, file) {
            generated.push(tokens);
        }
    }

    quote! {
        #(#generated)*
    }
}

fn generate_module(module_name: &str, is_active: bool, file: syn::File) -> Option<TokenStream2> {
    let mut exported_items = Vec::new();
    let mut sim_modules = Vec::new();

    for item in file.items {
        if has_sim_export_attr(item.attrs()) {
            let mut item = item;
            strip_sim_attrs_from_item(&mut item);
            exported_items.push(item);
            continue;
        }

        if is_active && has_sim_test_attr(item.attrs()) {
            let syn::Item::Mod(item_mod) = item else {
                panic!("#[es_sim::test] is only supported on modules at the file root");
            };

            let sim_mod = prepare_sim_module(item_mod, module_name);
            sim_modules.push(sim_mod);
        }
    }

    if exported_items.is_empty() && sim_modules.is_empty() {
        return None;
    }

    let module_ident = format_ident!("{module_name}");

    Some(quote! {
        #[allow(dead_code)]
        mod #module_ident {
            #(#exported_items)*
            #(#sim_modules)*
        }
    })
}

fn prepare_sim_module(item_mod: syn::ItemMod, module_name: &str) -> syn::ItemMod {
    let Some((_, mut items)) = item_mod.content else {
        panic!("#[es_sim::test] modules must be inline");
    };

    let mut wrappers = Vec::new();

    for item in items.iter_mut() {
        if let syn::Item::Fn(item_fn) = item {
            if let Some(attr) = take_sim_test_attr(&mut item_fn.attrs) {
                let config = parse_case_config(&attr);
                let case_ident = item_fn.sig.ident.clone();
                let wrapper_ident = format_ident!("__es_sim_{}", case_ident);
                let timeout_expr =
                    config.timeout_ms.unwrap_or_else(|| syn::parse_quote!(crate::CaseConfig::DEFAULT_TIMEOUT_MS));
                let boot_expr =
                    config.boot_ms.unwrap_or_else(|| syn::parse_quote!(crate::CaseConfig::DEFAULT_BOOT_MS));
                let reset_expr =
                    config.reset.unwrap_or_else(|| syn::parse_quote!(crate::CaseConfig::DEFAULT_RESET));
                let case_name = case_ident.to_string();
                let module_name = module_name.to_owned();

                wrappers.push(syn::parse_quote! {
                    #[test]
                    fn #wrapper_ident() {
                        crate::run_case(
                            #module_name,
                            #case_name,
                            crate::CaseConfig {
                                timeout_ms: #timeout_expr,
                                boot_ms: #boot_expr,
                                reset: #reset_expr,
                            },
                            #case_ident,
                        );
                    }
                });
            }
        }

        strip_sim_attrs_from_item(item);
    }

    items.extend(wrappers);

    syn::ItemMod {
        attrs: strip_sim_attrs(item_mod.attrs),
        content: Some((Default::default(), items)),
        ..item_mod
    }
}

#[derive(Default)]
struct CaseConfigExprs {
    timeout_ms: Option<syn::Expr>,
    boot_ms: Option<syn::Expr>,
    reset: Option<syn::Expr>,
}

fn parse_case_config(attr: &syn::Attribute) -> CaseConfigExprs {
    let mut config = CaseConfigExprs::default();

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("timeout_ms") {
            config.timeout_ms = Some(meta.value()?.parse::<syn::Expr>()?);
            return Ok(());
        }

        if meta.path.is_ident("boot_ms") {
            config.boot_ms = Some(meta.value()?.parse::<syn::Expr>()?);
            return Ok(());
        }

        if meta.path.is_ident("reset") {
            config.reset = Some(meta.value()?.parse::<syn::Expr>()?);
            return Ok(());
        }

        Err(meta.error("unsupported #[es_sim::test(...)] argument"))
    })
    .expect("failed to parse #[es_sim::test(...)]");

    config
}

fn has_sim_export_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| is_attr_path(attr, &["es_sim", "export"]))
}

fn has_sim_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| is_attr_path(attr, &["es_sim", "test"]))
}

fn take_sim_test_attr(attrs: &mut Vec<syn::Attribute>) -> Option<syn::Attribute> {
    let index = attrs.iter().position(|attr| is_attr_path(attr, &["es_sim", "test"]))?;
    Some(attrs.remove(index))
}

fn strip_sim_attrs(attrs: Vec<syn::Attribute>) -> Vec<syn::Attribute> {
    attrs.into_iter()
        .filter(|attr| {
            !is_attr_path(attr, &["es_sim", "test"]) && !is_attr_path(attr, &["es_sim", "export"])
        })
        .collect()
}

fn strip_sim_attrs_from_item(item: &mut syn::Item) {
    match item {
        syn::Item::Const(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Enum(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Fn(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Impl(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Mod(item_mod) => {
            item_mod.attrs = strip_sim_attrs(std::mem::take(&mut item_mod.attrs));
            if let Some((_, items)) = &mut item_mod.content {
                for nested in items {
                    strip_sim_attrs_from_item(nested);
                }
            }
        }
        syn::Item::Static(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Struct(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Trait(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Type(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Union(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        syn::Item::Use(item) => item.attrs = strip_sim_attrs(std::mem::take(&mut item.attrs)),
        _ => {}
    }
}

trait ItemAttrs {
    fn attrs(&self) -> &[syn::Attribute];
}

impl ItemAttrs for syn::Item {
    fn attrs(&self) -> &[syn::Attribute] {
        match self {
            syn::Item::Const(item) => &item.attrs,
            syn::Item::Enum(item) => &item.attrs,
            syn::Item::ExternCrate(item) => &item.attrs,
            syn::Item::Fn(item) => &item.attrs,
            syn::Item::ForeignMod(item) => &item.attrs,
            syn::Item::Impl(item) => &item.attrs,
            syn::Item::Macro(item) => &item.attrs,
            syn::Item::Mod(item) => &item.attrs,
            syn::Item::Static(item) => &item.attrs,
            syn::Item::Struct(item) => &item.attrs,
            syn::Item::Trait(item) => &item.attrs,
            syn::Item::TraitAlias(item) => &item.attrs,
            syn::Item::Type(item) => &item.attrs,
            syn::Item::Union(item) => &item.attrs,
            syn::Item::Use(item) => &item.attrs,
            _ => &[],
        }
    }
}

fn is_attr_path(attr: &syn::Attribute, expected: &[&str]) -> bool {
    let segments: Vec<_> = attr
        .path()
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    segments == expected
}
