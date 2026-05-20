//! go language plugin.

mod call_sites;
mod calls;
mod chain;
mod embedded;
mod refs;
mod flow;
mod flow_detectors;
mod helpers;
pub(crate) mod keywords;
mod symbols;
mod statements;
mod tags;
mod type_refs;
mod types;
pub mod extract;

pub mod hooks;
mod predicates;
pub mod profile;
pub(crate) mod type_checker;
pub mod connectors;

pub use hooks::GO_HOOKS;
pub use hooks::GoResolver;
pub use profile::GO_PROFILE;

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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&GO_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&GO_HOOKS)
    }

    // TODO(routes-dispatch): wire `connectors::discover_go_routes` into the
    // indexer route-population stage. The function now writes the `routes` table
    // directly (returning the insert count) and the routes-table → FlowEmission
    // bridge in resolve/mod.rs emits the Consumer flows. The
    // `resolve_connection_points` override was removed because the ConnectionPoint
    // Stop emission was redundant with that bridge.

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