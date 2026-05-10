//! Groovy language plugin.
//!
//! Covers `.groovy` and `.gradle` files.
//!
//! What we extract:
//! - `class_declaration` → Class
//! - `function_definition` → Function (top-level `def`)
//! - `method_declaration` → Method (typed, inside class body)
//! - `package_declaration` → Namespace
//! - `import_declaration` → Imports
//! - `method_invocation` → Calls

pub(crate) mod connectors;
pub(crate) mod keywords;
pub mod extract;

mod predicates;
pub(crate) mod type_checker;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GroovyPlugin;

impl LanguagePlugin for GroovyPlugin {
    fn id(&self) -> &str { "groovy" }

    fn language_ids(&self) -> &[&str] { &["groovy"] }

    fn extensions(&self) -> &[&str] { &[".groovy", ".gradle"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_groovy::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_groovy_connection_points(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "function_definition",
            "method_declaration",
            "package_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "method_invocation",
            "import_declaration",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::GroovyResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![]
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::GroovyChecker))
    }
}
