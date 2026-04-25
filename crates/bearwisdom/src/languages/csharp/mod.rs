//! csharp language plugin.

mod calls;
pub mod connectors;
pub(crate) mod decorators;
mod embedded;
mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;
mod types;
pub mod extract;

mod predicates;
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
use crate::types::{EmbeddedRegion, ExtractionResult};
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

    fn extract_connection_points(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        let mut out = Vec::new();
        connectors::extract_csharp_mq_src(source, &mut out);
        out
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

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::CSharpResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        // MQ migrated to source-scan via `extract_connection_points`.
        // DI / EventBus / gRPC / GraphQL / REST need DB joins and run
        // from `resolve_connection_points` below.
        vec![Box::new(connectors::CSharpMqConnector)]
    }

    fn resolve_connection_points(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        ctx: &crate::indexer::project_context::ProjectContext,
    ) -> Vec<crate::connectors::types::ConnectionPoint> {
        let mut out = Vec::new();
        out.extend(crate::languages::drive_connector(
            &connectors::DotnetDiConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::EventBusConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::CSharpGrpcConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::CSharpGraphQlConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::CsharpRestConnector, db, project_root, ctx,
        ));
        out
    }

    fn resolve_connection_points_incremental(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        ctx: &crate::indexer::project_context::ProjectContext,
        changed_paths: &std::collections::HashSet<String>,
    ) -> Vec<crate::connectors::types::ConnectionPoint> {
        // DI + event-bus scans the disk; scope to `changed_paths` so we
        // don't read 10k .cs files on every save. The DB-only scans
        // (gRPC, GraphQL, REST attribute-based) stay full-scope — they
        // only do indexed JOINs and need cross-file coverage.
        let mut out = Vec::new();
        out.extend(crate::languages::drive_connector_incremental(
            &connectors::DotnetDiConnector, db, project_root, ctx, changed_paths,
        ));
        out.extend(crate::languages::drive_connector_incremental(
            &connectors::EventBusConnector, db, project_root, ctx, changed_paths,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::CSharpGrpcConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::CSharpGraphQlConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::CsharpRestConnector, db, project_root, ctx,
        ));
        out
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        _project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        if let Err(e) = connectors::run_ef_core(db) {
            tracing::warn!("EF Core post-index hook: {e}");
        }
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::CSHARP_FLOW_CONFIG)
    }
}
