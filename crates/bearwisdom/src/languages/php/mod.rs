//! php language plugin.

mod calls;
pub(crate) mod decorators;
pub mod embedded;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod symbols;

pub mod connectors;
pub(crate) mod hooks;
mod predicates;
pub(crate) mod profile;

pub use hooks::PHP_HOOKS;
pub use profile::PHP_PROFILE;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

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
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct PhpPlugin;

impl LanguagePlugin for PhpPlugin {
    fn id(&self) -> &str {
        "php"
    }

    fn language_ids(&self) -> &[&str] {
        &["php"]
    }

    fn extensions(&self) -> &[&str] {
        &[".php"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_php::LANGUAGE_PHP.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::PHP_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PHP_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&hooks::PHP_HOOKS)
    }

    // TODO(routes-dispatch): wire `connectors::discover_laravel_routes` into
    // the indexer route-population stage. The function now writes the `routes`
    // table directly (returning the insert count) and the routes-table →
    // FlowEmission bridge in resolve/mod.rs emits the Consumer flows. The
    // `resolve_connection_points` override was removed because the
    // ConnectionPoint Stop emission was redundant with that bridge.

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::PHP_FLOW_CONFIG)
    }
}
