//! kotlin language plugin.

mod calls;
pub mod connectors;
pub(crate) mod decorators;
mod embedded;
mod helpers;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

pub use resolve::KotlinResolver;

pub struct KotlinPlugin;

impl LanguagePlugin for KotlinPlugin {
    fn id(&self) -> &str { "kotlin" }

    fn language_ids(&self) -> &[&str] { &["kotlin"] }

    fn extensions(&self) -> &[&str] { &[".kt", ".kts"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_kotlin_ng::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::KOTLIN_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "object_declaration",
            "companion_object",
            "function_declaration",
            "secondary_constructor",
            "primary_constructor",
            "property_declaration",
            "getter",
            "setter",
            "type_alias",
            "enum_entry",
            "class_parameter",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "constructor_invocation",
            "import_header",
            "delegation_specifier",
            "user_type",
            "nullable_type",
            "type_arguments",
            "as_expression",
            "check_expression",
            "annotation",
            "navigation_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::KotlinResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![
            Box::new(connectors::KotlinGrpcConnector),
            Box::new(connectors::KotlinMqConnector),
            Box::new(connectors::KotlinRestConnector),
        ]
    }
}