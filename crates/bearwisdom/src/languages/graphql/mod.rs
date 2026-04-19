//! GraphQL language plugin.

pub mod connectors;
pub mod keywords;
pub mod extract;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GraphQlPlugin;

impl LanguagePlugin for GraphQlPlugin {
    fn id(&self) -> &str {
        "graphql"
    }

    fn language_ids(&self) -> &[&str] {
        &["graphql"]
    }

    fn extensions(&self) -> &[&str] {
        &[".graphql", ".gql"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_graphql::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source, tree_sitter_graphql::LANGUAGE.into())
    }

    fn extract_connection_points(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_schema_starts(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "object_type_definition",
            "interface_type_definition",
            "enum_type_definition",
            "union_type_definition",
            "scalar_type_definition",
            "input_object_type_definition",
            "directive_definition",
            "schema_definition",
            "operation_definition",
            "fragment_definition",
            "field_definition",
            "enum_value_definition",
            "input_value_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "named_type",
            "implements_interfaces",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[
            "String", "Int", "Float", "Boolean", "ID",
        ]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::GraphQlResolver))
    }

    fn connectors(&self) -> Vec<Box<dyn crate::connectors::traits::Connector>> {
        vec![Box::new(connectors::GraphQlSchemaConnector)]
    }
}
