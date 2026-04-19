//! go language plugin.

mod calls;
mod embedded;
mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;
mod tags;
pub mod extract;

mod predicates;
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
use crate::types::{EmbeddedRegion, ExtractionResult};
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

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_go_connection_points(source, file_path)
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

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
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

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        // Disabled — go-pocketbase reproducibly OOMs on a 400MB single
        // allocation even though no source file is >130KB. The size guard
        // doesn't help; the cost is in tree-sitter query-automaton state
        // expansion on certain Go patterns (assignment_query's
        // `right: (expression_list (_) @rhs)` is suspect — unrestricted
        // wildcard inside a list creates combinatorial captures).
        //
        // Chain-walker gains from Sprint 1 (call-site type_args via
        // TypeEnvironment) still apply — the +14227-edge win on
        // go-pocketbase in earlier runs came with flow_config=None.
        None
    }
}