//! TypeScript / TSX / JavaScript / JSX language plugin.
//!
//! Handles extraction for all four language IDs. The TypeScript and JavaScript
//! grammars are separate tree-sitter crates but share most extraction logic.
//! TSX and JSX use their respective grammars for JSX support.

// Extraction sub-modules
pub mod connectors;
mod calls;
pub(crate) mod decorators;
mod embedded;
mod flow;
mod helpers;
mod imports;
mod narrowing;
mod params;
pub(crate) mod keywords;
mod symbols;
mod types;

pub mod extract;

// Resolution sub-modules
pub(crate) mod predicates;
pub(crate) mod type_checker;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

// Re-export the resolver for registration in default_resolvers().
pub use resolve::TypeScriptResolver;

/// TypeScript language plugin — handles "typescript", "tsx", "javascript", "jsx".
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn id(&self) -> &str {
        "typescript"
    }

    fn language_ids(&self) -> &[&str] {
        &["typescript", "tsx"]
    }

    fn extensions(&self) -> &[&str] {
        &[".ts", ".tsx", ".mts", ".cts"]
    }

    /// `.tsx` uses the TSX grammar, so the language id must be "tsx" (not the
    /// plugin's primary "typescript") for `grammar(lang_id)` to pick the
    /// right parser. Other extensions route to the TypeScript grammar.
    fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
        match ext.to_ascii_lowercase().as_str() {
            ".tsx" => Some("tsx"),
            ".ts" | ".mts" | ".cts" => Some("typescript"),
            _ => None,
        }
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        Some(match lang_id {
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => return None,
        })
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::TS_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let is_tsx = file_path.ends_with(".tsx") || lang_id == "tsx";
        extract::extract(source, is_tsx)
    }

    fn extract_connection_points(
        &self,
        source: &str,
        file_path: &str,
        _lang_id: &str,
    ) -> Vec<crate::types::ConnectionPoint> {
        connectors::extract_typescript_connection_points(source, file_path)
    }

    fn extract_with_demand(
        &self,
        source: &str,
        file_path: &str,
        lang_id: &str,
        demand: Option<&std::collections::HashSet<String>>,
    ) -> ExtractionResult {
        let is_tsx = file_path.ends_with(".tsx") || lang_id == "tsx";
        extract::extract_with_demand(source, is_tsx, demand)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source, lang_id)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration", "abstract_class_declaration",
            "interface_declaration",
            "function_declaration", "generator_function_declaration",
            "method_definition", "abstract_method_signature", "method_signature",
            "public_field_definition", "property_signature", "field_definition",
            "type_alias_declaration",
            "enum_declaration",
            "lexical_declaration", "variable_declaration",
            "internal_module",
            "construct_signature", "call_signature", "index_signature",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "import_statement",
            // jsx_self_closing_element and jsx_opening_element are intentionally excluded:
            // we only emit refs for PascalCase component tags (~23% of occurrences),
            // not HTML intrinsics (div, span, etc.), so the 1:1 node→ref assumption breaks.
            "extends_clause", "implements_clause",
            "type_annotation", "type_identifier",
            "as_expression", "satisfies_expression",
            "tagged_template_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::TypeScriptResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::TypeScriptChecker))
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
            &connectors::NestjsRouteConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::NextjsRouteConnector, db, project_root, ctx,
        ));
        out.extend(crate::languages::drive_connector(
            &connectors::TypeScriptRestConnector, db, project_root, ctx,
        ));
        out
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        connectors::run_react_patterns(db.conn(), project_root);
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        // Temporarily disabled while investigating ts-immich hang.
        // Set BW_TS_FLOW=1 to re-enable.
        if std::env::var_os("BW_TS_FLOW").is_some() {
            Some(&flow::TS_FLOW_CONFIG)
        } else {
            None
        }
    }
}
