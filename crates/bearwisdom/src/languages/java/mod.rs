//! java language plugin.

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
pub(crate) mod externals;
mod helpers;
pub(crate) mod primitives;
mod symbols;
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

pub use resolve::JavaResolver;

pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn id(&self) -> &str { "java" }

    fn language_ids(&self) -> &[&str] { &["java"] }

    fn extensions(&self) -> &[&str] { &[".java"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_java::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::JAVA_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "enum_constant",
            "record_declaration",
            "annotation_type_declaration",
            "method_declaration",
            "constructor_declaration",
            "compact_constructor_declaration",
            "field_declaration",
            "constant_declaration",
            "annotation_type_element_declaration",
            "package_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "method_invocation",
            "object_creation_expression",
            "import_declaration",
            "type_arguments",
            "instanceof_expression",
            "method_reference",
            "cast_expression",
            "annotation",
            "marker_annotation",
            "superclass",
            "super_interfaces",
            "extends_interfaces",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &["int", "long", "short", "byte", "float", "double", "boolean", "char", "void", "var"]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn framework_globals(&self, dependencies: &std::collections::HashSet<String>) -> Vec<&'static str> {
        externals::framework_globals(dependencies)
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::JavaResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![
            Box::new(connectors::SpringRouteConnector),
            Box::new(connectors::SpringDiConnector),
        ]
    }
}