//! ruby language plugin.
mod callback_lexical;

pub(crate) mod callback_contract;
mod calls;
mod external_virtual_path;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
pub(crate) mod package_specifier;
mod params;
mod rbi;
mod rbs;
mod symbols;

pub mod connectors;
mod predicates;
pub(crate) mod profile;
pub use profile::RUBY_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "rbs_tests.rs"]
mod rbs_tests;

#[cfg(test)]
#[path = "rbi_tests.rs"]
mod rbi_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct RubyPlugin;

impl LanguagePlugin for RubyPlugin {
    fn id(&self) -> &str {
        "ruby"
    }

    fn language_ids(&self) -> &[&str] {
        &["ruby", "rbi", "rbs"]
    }

    fn extensions(&self) -> &[&str] {
        &[".rb", ".rake", ".gemspec", ".rbi", ".rbs"]
    }

    fn language_id_for_extension(&self, ext: &str) -> Option<&str> {
        match ext.to_ascii_lowercase().as_str() {
            ".rb" | ".rake" | ".gemspec" => Some("ruby"),
            ".rbi" => Some("rbi"),
            ".rbs" => Some("rbs"),
            _ => None,
        }
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        matches!(lang_id, "ruby" | "rbi").then(|| tree_sitter_ruby::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::RUBY_SCOPE_KINDS
    }

    fn external_virtual_path(
        &self,
        _language: &str,
        normalized_absolute_path: &str,
    ) -> Option<String> {
        external_virtual_path::for_pulled(normalized_absolute_path)
    }

    fn callback_lexical_adapter(
        &self,
    ) -> Option<&'static crate::indexer::callback_lexical::CallbackLexicalAdapter> {
        Some(&callback_lexical::ADAPTER)
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = file_path;
        match lang_id {
            "ruby" => extract::extract(source),
            "rbi" => rbi::extract(source),
            "rbs" => rbs::extract(source),
            _ => ExtractionResult::default(),
        }
    }

    fn callback_argument_policy(
        &self,
        file_path: &str,
        signature: Option<&str>,
        index: usize,
    ) -> Option<crate::type_checker::profile::chain_specs::CallbackArgumentPolicy> {
        Some(callback_contract::argument_policy(
            file_path, signature, index,
        ))
    }

    fn signature_return_type(&self, signature: &str) -> Option<String> {
        crate::languages::colon_return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        crate::languages::colon_parameter_types(signature)
    }

    fn signature_declared_type(&self, signature: &str) -> Option<String> {
        crate::type_checker::profile::signature_parser::parse_declared_type_from_signature(
            signature,
            crate::type_checker::profile::signature_parser::DeclaredTypeLayout::AfterMarker(':'),
        )
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class",
            "module",
            "method",
            "singleton_method",
            "singleton_class",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["call", "scope_resolution", "constant"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::RUBY_PROFILE)
    }

    fn discover_routes(
        &self,
        conn: &rusqlite::Connection,
        project_root: &std::path::Path,
        project_ctx: &crate::indexer::project_context::ProjectContext,
    ) -> u32 {
        connectors::discover_rails_routes(conn, project_root, project_ctx)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::RUBY_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::RUBY_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::RUBY_RETURN_QUERY)
    }

    fn plugin_flow_emissions(
        &self,
        source: &str,
        file_path: &str,
    ) -> Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)> {
        connectors::extract_ruby_graphql(source, file_path)
    }
}
