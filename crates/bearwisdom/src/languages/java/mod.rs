//! java language plugin.

mod calls;
pub(crate) mod connectors;
pub(crate) mod decorators;
mod embedded;
mod flow;
mod flow_detectors;
mod helpers;
pub(crate) mod keywords;
mod lombok;
mod symbols;
pub mod extract;

pub mod hooks;
mod predicates;
pub mod profile;
pub use hooks::JAVA_HOOKS;
pub use hooks::JavaResolver;
pub use profile::JAVA_PROFILE;

#[cfg(test)]
#[path = "calls_tests.rs"]
mod calls_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

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
use crate::types::{EmbeddedRegion, ExtractedRef, ExtractedSymbol, ExtractionResult};
use crate::parser::scope_tree::ScopeKind;

pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn id(&self) -> &str { "java" }

    fn language_ids(&self) -> &[&str] { &["java"] }

    fn extensions(&self) -> &[&str] { &[".java"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_java::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::JAVA_SCOPE_KINDS }

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

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&JAVA_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&JAVA_HOOKS)
    }

    fn flow_config(&self) -> Option<&'static crate::indexer::flow::FlowConfig> {
        Some(&flow::JAVA_FLOW_CONFIG)
    }
}