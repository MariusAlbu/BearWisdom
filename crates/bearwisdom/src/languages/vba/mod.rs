//! VBA (Visual Basic for Applications) language plugin.
//!
//! Grammar: no tree-sitter grammar available on crates.io.
//! Uses a line scanner over VBA source.

pub mod extract;

pub(crate) mod hooks;
mod keywords;
pub(crate) mod profile;

pub use hooks::VBA_HOOKS;
pub use profile::VBA_PROFILE;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct VbaPlugin;

impl LanguagePlugin for VbaPlugin {
    fn id(&self) -> &str {
        "vba"
    }

    fn language_ids(&self) -> &[&str] {
        &["vba"]
    }

    fn extensions(&self) -> &[&str] {
        &[".bas", ".cls", ".frm"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "sub_declaration",
            "function_declaration",
            "class_module",
            "property_declaration",
            "variable_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["call_statement"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::VBA_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&hooks::VBA_HOOKS)
    }
}
