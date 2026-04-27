//! java language plugin.

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod embedded;
mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub(crate) mod type_checker;
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
use crate::types::{EmbeddedRegion, ExtractionResult};
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

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_java_connection_points(source, file_path)
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

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::JavaResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::JavaChecker))
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
            &connectors::SpringRouteConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::SpringDiConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::JavaRestConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::JavaGrpcConnector, db, project_root, ctx,
        ));
        out
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::JAVA_FLOW_CONFIG)
    }
}