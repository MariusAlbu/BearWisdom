//! csharp language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod symbols;
mod types;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::CSharpResolver;

pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &str { "csharp" }

    fn language_ids(&self) -> &[&str] { &["csharp"] }

    fn extensions(&self) -> &[&str] { &[".cs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_c_sharp::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::CSHARP_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "struct_declaration",
            "record_declaration",
            "interface_declaration",
            "enum_declaration",
            "enum_member_declaration",
            "delegate_declaration",
            "event_declaration",
            "event_field_declaration",
            "method_declaration",
            "constructor_declaration",
            "destructor_declaration",
            "property_declaration",
            "indexer_declaration",
            "operator_declaration",
            "conversion_operator_declaration",
            "field_declaration",
            "local_function_statement",
            "namespace_declaration",
            "file_scoped_namespace_declaration",
            "accessor_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "invocation_expression",
            "object_creation_expression",
            "implicit_object_creation_expression",
            "using_directive",
            "base_list",
            "type_argument_list",
            "cast_expression",
            "is_expression",
            "as_expression",
            "typeof_expression",
            "attribute",
            "generic_name",
        ]
    }
}