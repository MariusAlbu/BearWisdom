//! go language plugin.
mod callback_lexical;

mod call_sites;
mod calls;
mod chain;
mod embedded;
mod external_virtual_path;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod param_symbols;
mod qualified_types;
mod refs;
mod signature;
mod statements;
mod symbols;
mod tags;
mod type_refs;
mod types;

pub mod connectors;
mod predicates;
pub mod profile;
pub use profile::GO_PROFILE;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn id(&self) -> &str {
        "go"
    }

    fn language_ids(&self) -> &[&str] {
        &["go"]
    }

    fn extensions(&self) -> &[&str] {
        &[".go"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_go::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn callback_lexical_adapter(
        &self,
    ) -> Option<&'static crate::indexer::callback_lexical::CallbackLexicalAdapter> {
        Some(&callback_lexical::ADAPTER)
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn external_virtual_path(
        &self,
        _language: &str,
        normalized_absolute_path: &str,
    ) -> Option<String> {
        external_virtual_path::for_pulled(normalized_absolute_path)
    }

    fn signature_return_type(&self, signature: &str) -> Option<String> {
        signature::return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        signature::parameter_types(signature)
    }

    fn signature_declared_type(&self, signature: &str) -> Option<String> {
        signature::declared_type(signature)
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

    fn signature_type_application(&self, text: &str) -> (String, Vec<String>) {
        crate::languages::bracket_type_application(text)
    }

    fn signature_type_head<'a>(&self, text: &'a str) -> &'a str {
        crate::languages::bracket_type_head(text)
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&GO_PROFILE)
    }

    // TODO(routes-dispatch): wire `connectors::discover_go_routes` into the
    // indexer route-population stage. The function now writes the `routes` table
    // directly (returning the insert count) and the routes-table → FlowEmission
    // bridge in resolve/mod.rs emits the Consumer flows. The
    // `resolve_connection_points` override was removed because the ConnectionPoint
    // Stop emission was redundant with that bridge.

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::GO_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::GO_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::GO_RETURN_QUERY)
    }

    fn augment_flow(
        &self,
        root: tree_sitter::Node,
        source: &[u8],
        symbols: &[crate::types::ExtractedSymbol],
        _refs: &[crate::types::ExtractedRef],
        meta: &mut crate::types::FlowMeta,
    ) {
        flow::bind_range_element_locals(&root, source, symbols, meta);
    }
}
