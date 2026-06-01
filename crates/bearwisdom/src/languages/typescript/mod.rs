//! TypeScript / TSX / JavaScript / JSX language plugin.
//!
//! Handles extraction for all four language IDs. The TypeScript and JavaScript
//! grammars are separate tree-sitter crates but share most extraction logic.
//! TSX and JSX use their respective grammars for JSX support.

// Extraction sub-modules
pub mod connectors;
mod connectors_graphql;
mod connectors_nestjs;
mod connectors_nextjs;
mod connectors_react;
mod calls;
pub(crate) mod decorators;
mod embedded;
pub(crate) mod flow;
mod helpers;
mod imports;
mod narrowing;
mod params;
pub(crate) mod keywords;
mod symbols;
mod symbols_casts;
mod symbols_fields;
mod symbols_variables;
mod types;
mod alias_classify;

pub mod extract;
mod reexports;
mod type_scan;
pub(crate) mod selectors;

// Resolution sub-modules
pub(crate) mod predicates;
pub mod profile;
pub mod hooks;
mod aliases;
pub(crate) mod flow_detectors;

pub use hooks::TYPESCRIPT_HOOKS;
pub use hooks::TypeScriptResolver;
pub use profile::TYPESCRIPT_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

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
        let mut result = extract::extract(source, is_tsx);
        crate::languages::common::append_ember_helper_default_export(file_path, source, &mut result);
        crate::languages::common::append_handlebars_register_helper_globals(source, &mut result);
        result
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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&TYPESCRIPT_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&TYPESCRIPT_HOOKS)
    }

    // TODO(routes-dispatch): wire `connectors::discover_nestjs_routes` and
    // `connectors::discover_nextjs_routes` into the indexer route-population
    // stage. Both functions now write the `routes` table directly (returning
    // the insert count) and the routes-table → FlowEmission bridge in
    // resolve/mod.rs emits the Consumer flows. The `resolve_connection_points`
    // override was removed because the ConnectionPoint Stop emission was
    // redundant with that bridge.

    fn post_index(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        connectors::run_react_patterns(db.conn(), project_root);
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::TS_FLOW_CONFIG)
    }
}
