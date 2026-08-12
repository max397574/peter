use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Data, DeriveInput, Fields, GenericArgument, Meta, PathArguments, Type, parse_macro_input,
};

fn doc_comment(attrs: &[syn::Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            let Meta::NameValue(nv) = &attr.meta else {
                return None;
            };
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
            else {
                return None;
            };
            Some(s.value().trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" "))
    }
}

/// Result of mapping one Rust type to its LuaCATS spelling: the type
/// text itself, plus - when the type wasn't a recognized primitive or
/// container - the syn::Type of the nested type it fell back to
/// referencing by name (e.g. `LuaType` in `lua_type: LuaType`). The
/// caller uses that to also pull in the nested type's own definition
/// (its `---@alias`/`---@class`) via LuaAnnotated, rather than emitting
/// a dangling reference to a class that's never defined.
struct MappedType {
    text: String,
    nested: Option<Type>,
}

/// Maps a Rust type (as written in source) to a LuaCATS type annotation.
/// This works on syntax, not resolved types - proc macros run before
/// type-checking - so it pattern-matches on the type's textual path
/// rather than anything semantic. Falls back to the type's own last path
/// segment (treated as a Lua class/alias name - the assumption being
/// that nested types also derive LuaAnnotated, enforced later via a
/// trait bound on the generated impl, which turns "forgot the derive"
/// into an ordinary compile error rather than a silently dangling
/// reference).
fn lua_type(ty: &Type) -> MappedType {
    let plain = |text: &str| MappedType {
        text: text.to_string(),
        nested: None,
    };

    let Type::Path(type_path) = ty else {
        return plain("any");
    };
    let Some(segment) = type_path.path.segments.last() else {
        return plain("any");
    };
    let name = segment.ident.to_string();

    let generic_args: Vec<&Type> = match &segment.arguments {
        PathArguments::AngleBracketed(args) => args
            .args
            .iter()
            .filter_map(|a| match a {
                GenericArgument::Type(t) => Some(t),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };

    match name.as_str() {
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" => plain("integer"),
        "f32" | "f64" => plain("number"),
        "bool" => plain("boolean"),
        "String" | "str" => plain("string"),
        "Option" => match generic_args.first() {
            Some(inner) => {
                let m = lua_type(inner);
                MappedType {
                    text: format!("{}?", m.text),
                    nested: m.nested,
                }
            }
            None => plain("any?"),
        },
        "Vec" | "VecDeque" => match generic_args.first() {
            Some(inner) => {
                let m = lua_type(inner);
                MappedType {
                    text: format!("{}[]", m.text),
                    nested: m.nested,
                }
            }
            None => plain("any[]"),
        },
        "HashMap" | "BTreeMap" => match (generic_args.first(), generic_args.get(1)) {
            (Some(k), Some(v)) => {
                // A nested type used as a map key or value is rare and
                // two-nested-types-per-field isn't worth plumbing
                // through MappedType's single `nested` slot - table
                // keys/values in practice are primitives, so this drops
                // any nested type found here rather than complicate the
                // shape for a case that likely won't come up.
                let (mk, mv) = (lua_type(k), lua_type(v));
                MappedType {
                    text: format!("table<{}, {}>", mk.text, mv.text),
                    nested: None,
                }
            }
            _ => plain("table<any, any>"),
        },
        "HashSet" | "BTreeSet" => match generic_args.first() {
            Some(inner) => {
                let m = lua_type(inner);
                MappedType {
                    text: format!("table<{}, boolean>", m.text),
                    nested: None,
                }
            }
            None => plain("table<any, boolean>"),
        },
        // Anything else (a user-defined struct or enum, e.g. a nested
        // config type or a C-like enum like LuaType): reference it by
        // name, and record it as `nested` so the caller also emits its
        // definition.
        _ => MappedType {
            text: name,
            nested: Some(ty.clone()),
        },
    }
}

/// Trait bound tokens for every nested type discovered while mapping a
/// struct's fields, e.g. `LuaType: LuaAnnotated`. Added to the generated
/// impl so a nested type that wasn't itself derived (LuaAnnotated) is a
/// clear compile error - "add the derive to LuaType too" - rather than a
/// dangling, never-defined class reference in the generated .lua file.
fn nested_bounds(nested: &[Type]) -> proc_macro2::TokenStream {
    let bounds = nested.iter().map(|ty| quote! { #ty: LuaAnnotated });
    quote! { #(#bounds,)* }
}

/// Calls made at runtime to pull in each nested type's own definition
/// (its `---@alias`/`---@class`) plus anything *it* in turn depends on
/// (recursing through its own `nested_defs()`), deduplicated later by
/// the caller so a type referenced from two fields isn't emitted twice.
fn nested_defs_calls(nested: &[Type]) -> proc_macro2::TokenStream {
    let calls = nested.iter().map(|ty| {
        let name = match ty {
            Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        }
        .unwrap_or_default();
        quote! {
            {
                defs.push(<#ty as LuaAnnotated>::lua_class_def(#name));
                defs.extend(<#ty as LuaAnnotated>::nested_defs());
            }
        }
    });
    quote! { #(#calls)* }
}

fn derive_struct(
    struct_ident: &syn::Ident,
    struct_doc: Option<String>,
    fields: &Fields,
) -> TokenStream {
    let Fields::Named(fields) = fields else {
        return syn::Error::new_spanned(
            struct_ident,
            "LuaAnnotated requires named fields (no tuple or unit structs)",
        )
        .to_compile_error()
        .into();
    };

    // One formatted ---@field line per field, plus the syn::Type of any
    // nested (non-primitive) type referenced along the way - collected
    // so its own definition can be pulled in too, instead of leaving a
    // dangling reference to a class that's never generated.
    let mut nested_types: Vec<Type> = Vec::new();
    let field_lines: Vec<String> = fields
        .named
        .iter()
        .map(|field| {
            let field_name = field
                .ident
                .as_ref()
                .expect("Fields::Named guarantees an ident")
                .to_string();
            let mapped = lua_type(&field.ty);
            if let Some(nested_ty) = mapped.nested {
                nested_types.push(nested_ty);
            }
            match doc_comment(&field.attrs) {
                Some(doc) => format!("---@field {field_name} {} {doc}", mapped.text),
                None => format!("---@field {field_name} {}", mapped.text),
            }
        })
        .collect();

    let class_doc_line = struct_doc.map(|d| format!("--- {d}\n")).unwrap_or_default();
    let fields_block = field_lines.join("\n");

    // "{{FULL_NAME}}" is a literal placeholder substituted at *runtime*
    // (not macro-expansion time) via str::replace, not a second format!
    // call - a doc comment containing a stray '{' or '}' would otherwise
    // corrupt or panic a nested format! call. The same CwdConfig type
    // doesn't know it'll be registered as "Cwd"; the registry supplies
    // the component's name and composes the full "MyPrompt.Cwd.Config"
    // class name when it calls lua_class_def.
    let template = format!("{class_doc_line}---@class {{{{FULL_NAME}}}}\n{fields_block}");

    let bounds = nested_bounds(&nested_types);
    let defs_calls = nested_defs_calls(&nested_types);
    let where_clause = if nested_types.is_empty() {
        quote! {}
    } else {
        quote! { where #bounds }
    };

    let expanded = quote! {
        impl LuaAnnotated for #struct_ident #where_clause {
            fn lua_class_def(full_name: &str) -> String {
                #template.replace("{{FULL_NAME}}", full_name)
            }

            fn nested_defs() -> Vec<String> {
                let mut defs: Vec<String> = Vec::new();
                #defs_calls
                defs
            }
        }
    };

    expanded.into()
}

fn derive_enum(
    struct_ident: &syn::Ident,
    struct_doc: Option<String>,
    data_enum: &syn::DataEnum,
) -> TokenStream {
    // Only C-like (unit) variants make sense as a LuaCATS string-literal
    // union - a data-carrying variant (e.g. `Foo(String)`) would need a
    // discriminated-union table shape this doesn't attempt, so it's
    // rejected here with a clear message rather than silently mishandled.
    let mut variant_names = Vec::new();
    for variant in &data_enum.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return syn::Error::new_spanned(
                variant,
                "LuaAnnotated on an enum only supports unit variants (no associated data) - \
                 it generates a ---@alias string union, matching how serde serializes a C-like enum",
            )
            .to_compile_error()
            .into();
        }
        variant_names.push(variant.ident.to_string());
    }

    let class_doc_line = struct_doc.map(|d| format!("--- {d}\n")).unwrap_or_default();
    let union = variant_names
        .iter()
        .map(|v| format!("\"{v}\""))
        .collect::<Vec<_>>()
        .join("|");
    let template = format!("{class_doc_line}---@alias {{{{FULL_NAME}}}} {union}");

    let expanded = quote! {
        impl LuaAnnotated for #struct_ident {
            fn lua_class_def(full_name: &str) -> String {
                #template.replace("{{FULL_NAME}}", full_name)
            }

            fn nested_defs() -> Vec<String> {
                Vec::new()
            }
        }
    };

    expanded.into()
}

#[proc_macro_derive(LuaAnnotated)]
pub fn derive_lua_annotated(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_ident = input.ident.clone();
    let struct_doc = doc_comment(&input.attrs);

    match &input.data {
        Data::Struct(data_struct) => derive_struct(&struct_ident, struct_doc, &data_struct.fields),
        Data::Enum(data_enum) => derive_enum(&struct_ident, struct_doc, data_enum),
        Data::Union(_) => {
            syn::Error::new_spanned(&input, "LuaAnnotated cannot be derived for unions")
                .to_compile_error()
                .into()
        }
    }
}
