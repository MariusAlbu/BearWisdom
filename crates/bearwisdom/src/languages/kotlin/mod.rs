//! kotlin language plugin.
mod callback_lexical;

mod calls;
mod data_class;
pub(crate) mod decorators;
mod embedded;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod predicates;
pub(crate) mod profile;
mod symbols;
pub use profile::KOTLIN_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "data_class_tests.rs"]
mod data_class_tests;

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod predicates_tests;

use crate::ecosystem::manifest::gradle::discover_gradle_catalog_names;
use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::project_context::ProjectContext;
use crate::languages::{LanguagePlugin, Synthesized};
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult, ParsedFile};

pub struct KotlinPlugin;

impl LanguagePlugin for KotlinPlugin {
    fn id(&self) -> &str {
        "kotlin"
    }

    fn language_ids(&self) -> &[&str] {
        &["kotlin"]
    }

    fn extensions(&self) -> &[&str] {
        &[".kt", ".kts"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_kotlin_ng::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::KOTLIN_SCOPE_KINDS
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

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn synthesize_symbols(
        &self,
        source: &str,
        symbols: &[ExtractedSymbol],
        refs: &[ExtractedRef],
    ) -> Synthesized {
        data_class::synthesize_data_class_members(source, symbols, refs)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "object_declaration",
            "companion_object",
            "function_declaration",
            "secondary_constructor",
            "primary_constructor",
            "property_declaration",
            "getter",
            "setter",
            "type_alias",
            "enum_entry",
            "class_parameter",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "constructor_invocation",
            "import_header",
            "delegation_specifier",
            "user_type",
            "nullable_type",
            "type_arguments",
            "as_expression",
            "check_expression",
            "annotation",
            "navigation_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn signature_type_application(&self, text: &str) -> (String, Vec<String>) {
        crate::languages::angle_type_application(text)
    }

    fn signature_type_head<'a>(&self, text: &'a str) -> &'a str {
        crate::languages::angle_type_head(text)
    }

    fn signature_return_type(&self, signature: &str) -> Option<String> {
        crate::languages::colon_return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        crate::languages::colon_parameter_types(signature)
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::KOTLIN_PROFILE)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::KOTLIN_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::KOTLIN_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::KOTLIN_RETURN_QUERY)
    }

    fn populate_project_state(
        &self,
        state: &mut PluginStateBag,
        _parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &ProjectContext,
    ) {
        state.set(discover_gradle_catalog_names(project_root));
    }
}
