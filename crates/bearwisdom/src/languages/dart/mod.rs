//! dart language plugin.
mod callback_lexical;

mod call_args;
mod call_sites;
mod calls;
pub(crate) mod decorators;
mod external_virtual_path;
pub mod extract;
pub(crate) mod flow;
mod helpers;
mod heritage;
mod imports;
pub(crate) mod keywords;
mod member_chain;
pub(crate) mod package_specifier;
mod predicates;
pub(crate) mod profile;
mod symbols;
pub use profile::DART_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &str {
        "dart"
    }

    fn language_ids(&self) -> &[&str] {
        &["dart"]
    }

    fn extensions(&self) -> &[&str] {
        &[".dart"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_dart::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
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

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "mixin_declaration",
            "enum_declaration",
            "enum_constant",
            "extension_declaration",
            "extension_type_declaration",
            "function_signature",
            "constructor_signature",
            "factory_constructor_signature",
            "getter_signature",
            "setter_signature",
            "initialized_variable_definition",
            "type_alias",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "new_expression",
            "const_object_expression",
            "constructor_invocation",
            "library_import",
            "library_export",
            "type_test_expression",
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
            dart_function: true,
            nullable_suffix: true,
            angle_application: true,
            ..crate::languages::TypeTextPolicy::OPAQUE
        }
    }

    fn source_module_path_policy(
        &self,
        _specifier: &str,
    ) -> crate::type_checker::profile::language_profile::SourceModulePathPolicy {
        predicates::SOURCE_MODULE_PATH_POLICY
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::DART_PROFILE)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::DART_FLOW_CONFIG)
    }

    fn normalize_flow_guard_type(&self, raw: &str) -> Option<String> {
        crate::languages::common::normalize_identifier_capture(raw)
    }

    fn flow_cfg_node_kinds(&self) -> Option<&'static crate::indexer::flow_cfg::CfgNodeKinds> {
        Some(&flow::DART_CFG_KINDS)
    }

    // DartRestConnector deleted — its routes-table re-read for Stop points
    // is redundant with the routes-table → FlowEmission bridge in
    // `indexer/resolve/mod.rs::append_db_route_consumer_emissions`.
    // Start points still flow through `extract_connection_points`.
}
