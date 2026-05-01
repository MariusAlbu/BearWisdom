//! php language plugin.

mod calls;
pub(crate) mod decorators;
pub mod embedded;
mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub(crate) mod type_checker;
pub mod connectors;
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

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
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

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_php_connection_points(source, file_path)
    }

    /// E2: surface `<script>` and `<style>` blocks that live in the HTML
    /// regions between `<?php … ?>` blocks for sub-extraction by the JS,
    /// TS, CSS, and SCSS plugins. Pure-PHP files (no HTML mode) emit
    /// nothing.
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

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PhpResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::PhpChecker))
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
        let mut out = Vec::new();
        out.extend(crate::languages::drive_connector(
            &connectors::LaravelRouteConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::PhpRestConnector, db, project_root, ctx,
        ));
        out
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::PHP_FLOW_CONFIG)
    }
}