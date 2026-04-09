//! go language plugin.

mod calls;
mod helpers;
pub(crate) mod primitives;
mod symbols;
mod tags;
pub mod extract;

mod builtins;
mod chain;
pub mod connectors;
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

pub use resolve::GoResolver;

pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn id(&self) -> &str { "go" }

    fn language_ids(&self) -> &[&str] { &["go"] }

    fn extensions(&self) -> &[&str] { &[".go"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_go::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_declaration",
            "method_declaration",
            "type_spec",
            "type_alias",
            "const_spec",
            "var_spec",
            "field_declaration",
            "method_elem",
            "package_clause",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "import_spec",
            "composite_literal",
            "type_conversion_expression",
            "type_assertion_expression",
            "selector_expression",
            "qualified_type",
            "type_identifier",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &["int", "int8", "int16", "int32", "int64", "uint", "uint8", "uint16", "uint32", "uint64", "float32", "float64", "complex64", "complex128", "string", "bool", "byte", "rune", "error", "any", "comparable", "uintptr"]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::GoResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![
            Box::new(connectors::GoRouteConnector),
            Box::new(connectors::GoRestConnector),
            Box::new(connectors::GoGrpcConnector),
            Box::new(connectors::GoMqConnector),
        ]
    }
}