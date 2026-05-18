//! Proc-macros for `codeless-plugin-sdk`.
//!
//! Just one for now: `#[derive(Tool)]`, which reads a
//! `#[tool(id = "...", tier = "...", description = "...")]`
//! attribute off a struct and emits the [`ToolMeta`] impl the SDK's
//! [`Manifest::for_behavior`] reads. The author still writes the
//! `impl ToolBehavior for MyTool { ... }` body by hand; the derive
//! only fills in the compile-time constants.
//!
//! Lifted pattern (not lifted code) from
//! `rubix-extensions-sdk-macros::NodeKind`. Rubix's derive reads a
//! YAML manifest at compile time and embeds the whole `KindManifest`
//! struct; codeless's tool manifest is built from `schemars`-derived
//! schemas at runtime so the derive only needs the literal id, tier,
//! and description -- a smaller surface that does not need an
//! `include_str!` of an out-of-line file.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{parse_macro_input, Attribute, DeriveInput, Expr, ExprLit, Lit, Meta, Token};

/// Derive `ToolMeta` for a struct from a `#[tool(...)]` attribute.
///
/// Required keys:
///
/// - `id = "<dotted.tool.id>"` -- the MCP-visible tool id. The
///   persona-allowed-tools matcher in `codeless-types` is
///   dotted-prefix-glob only (`notes.*` matches `notes.append`).
/// - `tier = "read" | "write" | "destructive"` -- maps to
///   [`codeless_plugin_sdk::Tier`].
///
/// Optional:
///
/// - `description = "..."` -- human-readable summary. Defaults to
///   the empty string. Used by `codeless plugin info` and surfaces
///   to the LLM through the MCP tool advertisement.
///
/// Unknown keys are a compile error so a typo in the attribute does
/// not silently expand to a half-empty manifest.
#[proc_macro_derive(Tool, attributes(tool))]
pub fn derive_tool(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);
    match expand(&ast) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

struct ToolAttrs {
    id: String,
    tier: String,
    description: String,
}

fn expand(ast: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ty = &ast.ident;
    let attrs = parse_tool_attrs(&ast.attrs)?;

    let id = attrs.id;
    let description = attrs.description;
    let tier_ident = match attrs.tier.as_str() {
        "read" => quote!(Read),
        "write" => quote!(Write),
        "destructive" => quote!(Destructive),
        other => {
            return Err(syn::Error::new(
                Span::call_site(),
                format!("unknown tier `{other}` -- expected one of `read`, `write`, `destructive`"),
            ));
        }
    };

    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::codeless_plugin_sdk::ToolMeta for #ty #ty_generics #where_clause {
            const ID: &'static str = #id;
            const TIER: ::codeless_plugin_sdk::Tier =
                ::codeless_plugin_sdk::Tier::#tier_ident;
            const DESCRIPTION: &'static str = #description;
        }
    })
}

fn parse_tool_attrs(attrs: &[Attribute]) -> syn::Result<ToolAttrs> {
    let mut id: Option<String> = None;
    let mut tier: Option<String> = None;
    let mut description: Option<String> = None;
    let mut outer_span: Option<Span> = None;

    for attr in attrs {
        if !attr.path().is_ident("tool") {
            continue;
        }
        outer_span = Some(attr.path().segments[0].ident.span());
        let nested =
            attr.parse_args_with(syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for meta in nested {
            let Meta::NameValue(nv) = meta else {
                return Err(syn::Error::new_spanned(
                    meta,
                    "expected `key = \"value\"` inside `#[tool(...)]`",
                ));
            };
            let Expr::Lit(ExprLit {
                lit: Lit::Str(s), ..
            }) = &nv.value
            else {
                return Err(syn::Error::new_spanned(
                    &nv.value,
                    "expected a string literal",
                ));
            };
            let value = s.value();
            if nv.path.is_ident("id") {
                id = Some(value);
            } else if nv.path.is_ident("tier") {
                tier = Some(value);
            } else if nv.path.is_ident("description") {
                description = Some(value);
            } else {
                return Err(syn::Error::new_spanned(
                    &nv.path,
                    "unknown attribute -- expected one of `id`, `tier`, `description`",
                ));
            }
        }
    }

    let span = outer_span.unwrap_or_else(Span::call_site);
    let id = id.ok_or_else(|| {
        syn::Error::new(
            span,
            "missing `id = \"...\"` in `#[tool(...)]` -- the dotted MCP tool id",
        )
    })?;
    let tier = tier.ok_or_else(|| {
        syn::Error::new(
            span,
            "missing `tier = \"...\"` in `#[tool(...)]` -- one of \
             `read`, `write`, `destructive`",
        )
    })?;
    Ok(ToolAttrs {
        id,
        tier,
        description: description.unwrap_or_default(),
    })
}
