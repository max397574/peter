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

fn lua_type(ty: &Type) -> String {
    let Type::Path(type_path) = ty else {
        return "any".to_string();
    };
    let Some(segment) = type_path.path.segments.last() else {
        return "any".to_string();
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
        | "isize" => "integer".to_string(),
        "f32" | "f64" => "number".to_string(),
        "bool" => "boolean".to_string(),
        "String" | "str" => "string".to_string(),
        "Option" => match generic_args.first() {
            Some(inner) => format!("{}?", lua_type(inner)),
            None => "any?".to_string(),
        },
        "Vec" | "VecDeque" => match generic_args.first() {
            Some(inner) => format!("{}[]", lua_type(inner)),
            None => "any[]".to_string(),
        },
        "HashMap" | "BTreeMap" => match (generic_args.first(), generic_args.get(1)) {
            (Some(k), Some(v)) => format!("table<{}, {}>", lua_type(k), lua_type(v)),
            _ => "table<any, any>".to_string(),
        },
        "HashSet" | "BTreeSet" => match generic_args.first() {
            Some(inner) => format!("table<{}, boolean>", lua_type(inner)),
            None => "table<any, boolean>".to_string(),
        },
        other => other.to_string(),
    }
}

#[proc_macro_derive(LuaAnnotated)]
pub fn derive_lua_annotated(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_ident = &input.ident;
    let struct_doc = doc_comment(&input.attrs);

    let Data::Struct(data_struct) = &input.data else {
        return syn::Error::new_spanned(&input, "LuaAnnotated can only be derived for structs")
            .to_compile_error()
            .into();
    };
    let Fields::Named(fields) = &data_struct.fields else {
        return syn::Error::new_spanned(
            &input,
            "LuaAnnotated requires named fields (no tuple or unit structs)",
        )
        .to_compile_error()
        .into();
    };

    let field_lines: Vec<String> = fields
        .named
        .iter()
        .map(|field| {
            let field_name = field
                .ident
                .as_ref()
                .expect("Fields::Named guarantees an ident")
                .to_string();
            let ty = lua_type(&field.ty);
            match doc_comment(&field.attrs) {
                Some(doc) => format!("---@field {field_name} {ty} {doc}"),
                None => format!("---@field {field_name} {ty}"),
            }
        })
        .collect();

    let class_doc_line = struct_doc.map(|d| format!("--- {d}\n")).unwrap_or_default();
    let fields_block = field_lines.join("\n");

    let template = format!("{class_doc_line}---@class {{{{FULL_NAME}}}}\n{fields_block}");

    let expanded = quote! {
        impl LuaAnnotated for #struct_ident {
            fn lua_class_def(full_name: &str) -> String {
                #template.replace("{{FULL_NAME}}", full_name)
            }
        }
    };

    expanded.into()
}
