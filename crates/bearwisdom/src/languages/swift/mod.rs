//! swift language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod symbols;
pub mod extract;

mod builtins;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::SwiftResolver;

pub struct SwiftPlugin;

impl LanguagePlugin for SwiftPlugin {
    fn id(&self) -> &str { "swift" }

    fn language_ids(&self) -> &[&str] { &["swift"] }

    fn extensions(&self) -> &[&str] { &[".swift"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_swift::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::SWIFT_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "protocol_declaration",
            "enum_class_body",
            "function_declaration",
            "init_declaration",
            "protocol_function_declaration",
            "property_declaration",
            "protocol_property_declaration",
            "typealias_declaration",
            "subscript_declaration",
            "associatedtype_declaration",
            "operator_declaration",
            "enum_entry",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "constructor_expression",
            "import_declaration",
            "inheritance_specifier",
            "type_annotation",
            "user_type",
            "as_expression",
            "check_expression",
            "type_identifier",
            "protocol_composition_type",
        ]
    }
}