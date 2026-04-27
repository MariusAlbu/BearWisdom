//! dart language plugin.

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod helpers;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub mod resolve;
pub(crate) mod type_checker;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::DartResolver;

pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &str { "dart" }

    fn language_ids(&self) -> &[&str] { &["dart"] }

    fn extensions(&self) -> &[&str] { &[".dart"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_dart::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_dart_connection_points(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "mixin_declaration",
            "enum_declaration",
            "enum_constant",
            "extension_declaration",
            "extension_type_declaration",
            "function_signature",
            "constructor_signature",
            "factory_constructor_signature",
            "getter_signature",
            "setter_signature",
            "initialized_variable_definition",
            "type_alias",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "new_expression",
            "const_object_expression",
            "constructor_invocation",
            "library_import",
            "library_export",
            "type_test_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::DartResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::DartChecker))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![]
    }

    fn resolve_connection_points(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) -> Vec<crate::connectors::types::ConnectionPoint> {
        crate::languages::drive_connector(
            &connectors::DartRestConnector, db, project_root, ctx,
        )
    }

}