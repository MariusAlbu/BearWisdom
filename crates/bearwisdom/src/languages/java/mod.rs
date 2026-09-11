//! java language plugin.
mod callback_lexical;

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod embedded;
pub mod extract;
pub(crate) mod flow;
mod helpers;
pub(crate) mod keywords;
mod lombok;
mod predicates;
pub mod profile;
mod symbols;
pub use profile::JAVA_PROFILE;

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
#[path = "predicates_tests.rs"]
mod predicates_tests;

#[cfg(test)]
#[path = "lombok_tests.rs"]
mod lombok_tests;

use crate::languages::{LanguagePlugin, Synthesized};
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult};

pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn id(&self) -> &str {
        "java"
    }

    fn language_ids(&self) -> &[&str] {
        &["java"]
    }

    fn extensions(&self) -> &[&str] {
        &[".java"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_java::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::JAVA_SCOPE_KINDS
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

    fn signature_return_type(&self, signature: &str) -> Option<String> {
        crate::languages::prefix_return_type(signature)
    }

    fn signature_parameter_types(&self, signature: &str) -> Option<Vec<String>> {
        crate::languages::prefix_parameter_types(signature)
    }

    fn signature_declared_type(&self, signature: &str) -> Option<String> {
        crate::languages::prefix_declared_type(signature)
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
        lombok::synthesize_lombok_accessors(source, symbols, refs)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "enum_constant",
            "record_declaration",
            "annotation_type_declaration",
            "method_declaration",
            "constructor_declaration",
            "compact_constructor_declaration",
            "field_declaration",
            "constant_declaration",
            "annotation_type_element_declaration",
            "package_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "method_invocation",
            "object_creation_expression",
            "import_declaration",
            "type_arguments",
            "instanceof_expression",
            "method_reference",
            "cast_expression",
            "annotation",
            "marker_annotation",
            "superclass",
            "super_interfaces",
            "extends_interfaces",
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

    fn type_text_policy(&self) -> crate::languages::TypeTextPolicy {
        crate::languages::TypeTextPolicy {
            array_suffix: true,
            angle_application: true,
            ..crate::languages::TypeTextPolicy::OPAQUE
        }
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&JAVA_PROFILE)
    }

    fn discover_routes(
        &self,
        conn: &rusqlite::Connection,
        project_root: &std::path::Path,
        _project_ctx: &crate::indexer::project_context::ProjectContext,
    ) -> u32 {
        connectors::discover_spring_routes(conn, project_root)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::JAVA_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::JAVA_CFG_KINDS)
    }

    fn flow_return_query(&self) -> Option<&'static str> {
        Some(flow::JAVA_RETURN_QUERY)
    }
}
