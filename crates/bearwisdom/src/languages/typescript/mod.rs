//! TypeScript / TSX / JavaScript / JSX language plugin.
//!
//! Handles extraction for all four language IDs. The TypeScript and JavaScript
//! grammars are separate tree-sitter crates but share most extraction logic.
//! TSX and JSX use their respective grammars for JSX support.

// Extraction sub-modules
mod alias_classify;
mod alias_type_text;
mod alias_union;
mod annotation_members;
mod annotation_named_type;
mod calls;
pub mod connectors;
mod connectors_graphql;
mod connectors_nestjs;
mod connectors_nextjs;
mod connectors_react;
pub(crate) mod decorators;
mod embedded;
pub(crate) mod flow;
mod helpers;
mod imports;
pub(crate) mod keywords;
mod narrowing;
mod params;
mod symbols;
mod symbols_casts;
mod symbols_fields;
mod symbols_variables;
mod types;

pub mod extract;
mod reexports;
pub(crate) mod selectors;
mod type_scan;

// Resolution sub-modules
pub(crate) mod predicates;
pub mod profile;
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

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

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
        crate::languages::common::append_ember_helper_default_export(
            file_path,
            source,
            &mut result,
        );
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
            "class_declaration",
            "abstract_class_declaration",
            "interface_declaration",
            "function_declaration",
            "generator_function_declaration",
            "method_definition",
            "abstract_method_signature",
            "method_signature",
            "public_field_definition",
            "property_signature",
            "field_definition",
            "type_alias_declaration",
            "enum_declaration",
            "lexical_declaration",
            "variable_declaration",
            "internal_module",
            "construct_signature",
            "call_signature",
            "index_signature",
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
            "extends_clause",
            "implements_clause",
            "type_annotation",
            "type_identifier",
            "as_expression",
            "satisfies_expression",
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

    /// An Angular NgModule declaration `.d.ts` reaches the `.component`/`.directive`
    /// `.d.ts` files it declares — components/directives are referenced only by
    /// selector, so nothing demands them by name; descending the module's
    /// declarations is the structural signal that materializes them (and their
    /// `ɵcmp`/`ɵdir` selectors). Gated on the `ɵɵNgModuleDeclaration` marker, so
    /// non-Angular `.d.ts` cost nothing.
    fn external_declaration_reachables(&self, file_path: &str, content: &str) -> Vec<String> {
        if !file_path.ends_with(".module.d.ts") || !content.contains("ɵɵNgModuleDeclaration") {
            return Vec::new();
        }
        let mut out = Vec::new();
        for line in content.lines() {
            let t = line.trim();
            if !(t.starts_with("import ") || t.starts_with("export ")) {
                continue;
            }
            let Some(spec) = crate::ecosystem::npm::extract_quoted_after(t, " from ") else {
                continue;
            };
            if spec.starts_with('.') && (spec.contains(".component") || spec.contains(".directive"))
            {
                out.push(spec.to_string());
            }
        }
        out
    }
}
