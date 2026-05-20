//! COBOL language plugin.
//!
//! Grammar: tree-sitter-cobol 0.1.0 is a stub crate (binary-only, no library).
//! `grammar()` returns `None`; extraction uses a line-oriented scanner that
//! recognises COBOL's division/section/paragraph structure.

pub mod keywords;
pub mod extract;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;
pub(crate) mod type_checker;
pub(crate) mod resolve;

pub use hooks::COBOL_HOOKS;
pub use profile::COBOL_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct CobolPlugin;

impl LanguagePlugin for CobolPlugin {
    fn id(&self) -> &str {
        "cobol"
    }

    fn language_ids(&self) -> &[&str] {
        &["cobol"]
    }

    fn extensions(&self) -> &[&str] {
        &[".cob", ".cbl", ".cpy"]
    }

    /// tree-sitter-cobol 0.1.0 is a placeholder binary crate — no grammar available.
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
            "paragraph",
            "section",
            "data_description",
            "perform_statement",
            "call_statement",
            "copy_statement",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "perform_statement",
            "call_statement",
            "copy_statement",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] { keywords::KEYWORDS }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::CobolChecker))
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::COBOL_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::COBOL_HOOKS)
    }
}
