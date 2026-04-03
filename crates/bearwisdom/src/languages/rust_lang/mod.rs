//! rust_lang language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod patterns;
mod symbols;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::RustResolver;

pub struct RustLangPlugin;

impl LanguagePlugin for RustLangPlugin {
    fn id(&self) -> &str { "rust_lang" }

    fn language_ids(&self) -> &[&str] { &["rust"] }

    fn extensions(&self) -> &[&str] { &[".rs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_rust::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "struct_item",
            "enum_item",
            "enum_variant",
            "trait_item",
            "impl_item",
            "function_item",
            "function_signature_item",
            "const_item",
            "static_item",
            "type_item",
            "associated_type",
            "mod_item",
            "field_declaration",
            "union_item",
            "macro_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "macro_invocation",
            "struct_expression",
            "use_declaration",
            "impl_item",
            "type_cast_expression",
            "type_arguments",
            "attribute_item",
            "trait_bounds",
            "scoped_type_identifier",
            "type_identifier",
            "dynamic_type",
            "abstract_type",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &["bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "Self"]
    }
}