//! dart language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;
pub mod resolve;
pub(crate) mod type_checker;

pub use hooks::DART_HOOKS;
pub use profile::DART_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::DartResolver;

pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &str { "dart" }

    fn language_ids(&self) -> &[&str] { &["dart"] }

    fn extensions(&self) -> &[&str] { &[".dart"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_dart::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::DART_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::DART_HOOKS)
    }

    // DartRestConnector deleted — its routes-table re-read for Stop points
    // is redundant with the routes-table → FlowEmission bridge in
    // `indexer/resolve/mod.rs::append_db_route_consumer_emissions`.
    // Start points still flow through `extract_connection_points`.

}