//! swift language plugin.

mod calls;
pub(crate) mod decorators;
mod embedded;
mod helpers;
pub(crate) mod keywords;
mod symbols;
pub mod extract;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;

pub use hooks::SWIFT_HOOKS;
pub use profile::SWIFT_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::{EmbeddedRegion, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

pub struct SwiftPlugin;

impl LanguagePlugin for SwiftPlugin {
    fn id(&self) -> &str { "swift" }

    fn language_ids(&self) -> &[&str] { &["swift"] }

    fn extensions(&self) -> &[&str] { &[".swift"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_swift::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::SWIFT_SCOPE_KINDS }

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
            "class_declaration",
            "struct_declaration",
            "protocol_declaration",
            "enum_declaration",    // ← was incorrectly "enum_class_body" (the body container)
            "function_declaration",
            "init_declaration",
            "protocol_function_declaration",
            "property_declaration",
            "protocol_property_declaration",
            "typealias_declaration",
            "subscript_declaration",
            "associatedtype_declaration",
            "operator_declaration",
            "enum_entry",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "constructor_expression",
            "import_declaration",
            "inheritance_specifier",
            "type_annotation",
            "user_type",
            "as_expression",
            "check_expression",
            // type_identifier is intentionally excluded: in Swift's grammar, type_identifier
            // always appears as a leaf inside user_type. Counting both inflates the denominator
            // since the extractor emits one ref per user_type (matching the user_type budget),
            // leaving all type_identifier entries unmatched. user_type is the correct unit.
            "protocol_composition_type",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::SWIFT_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::SWIFT_HOOKS)
    }

    // resolve_connection_points removed — SwiftRestConnector's only role was
    // re-reading the `routes` table for Stop points, which the central
    // routes-table → FlowEmission bridge in `indexer/resolve/mod.rs` already
    // handles for every language. Start points still flow through
    // `extract_connection_points` from source-scan.
}