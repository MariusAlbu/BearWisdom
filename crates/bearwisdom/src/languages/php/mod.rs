//! php language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
pub(crate) mod primitives;
mod symbols;
pub mod extract;

mod builtins;
mod chain;
pub mod connectors;
pub(crate) mod externals;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::PhpResolver;

pub struct PhpPlugin;

impl LanguagePlugin for PhpPlugin {
    fn id(&self) -> &str { "php" }

    fn language_ids(&self) -> &[&str] { &["php"] }

    fn extensions(&self) -> &[&str] { &[".php"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_php::LANGUAGE_PHP.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::PHP_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "interface_declaration",
            "trait_declaration",
            "enum_declaration",
            "enum_case",
            "function_definition",
            "method_declaration",
            "property_declaration",
            "const_declaration",
            "namespace_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call_expression",
            "member_call_expression",
            "nullsafe_member_call_expression",
            "scoped_call_expression",
            "object_creation_expression",
            "namespace_use_declaration",
            "use_declaration",
            "base_clause",
            "class_interface_clause",
            "attribute",
            "named_type",
            "union_type",
            "intersection_type",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &["int", "float", "string", "bool", "void", "null", "array", "object", "mixed", "never", "callable", "iterable", "self", "static", "parent", "true", "false"]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn framework_globals(&self, deps: &std::collections::HashSet<String>) -> Vec<&'static str> {
        externals::framework_globals(deps)
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PhpResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![
            Box::new(connectors::LaravelRouteConnector),
            Box::new(connectors::PhpRestConnector),
        ]
    }
}