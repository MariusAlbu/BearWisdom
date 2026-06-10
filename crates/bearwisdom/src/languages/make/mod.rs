//! Make / Makefile language plugin.

pub mod extract;
pub(crate) mod hooks;
pub mod keywords;
pub(crate) mod profile;

pub use hooks::MAKE_HOOKS;
pub use profile::MAKE_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct MakePlugin;

impl LanguagePlugin for MakePlugin {
    fn id(&self) -> &str {
        "make"
    }

    fn language_ids(&self) -> &[&str] {
        &["make"]
    }

    /// Extensions for Make files. `Makefile` (no dot) is detected by
    /// bearwisdom-profile via filename matching.
    fn extensions(&self) -> &[&str] {
        &["Makefile", ".mk"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_make::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = file_path;
        // The 414-line tree-sitter-make extractor was sitting unused
        // because this plugin returned `empty()`. Wire the grammar and
        // delegate so rule targets, variable assignments, and include
        // directives reach the indexer.
        match self.grammar(lang_id) {
            Some(language) => extract::extract(source, language),
            None => ExtractionResult::empty(),
        }
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "rule",
            "variable_assignment",
            "define_directive",
            "shell_assignment",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["include_directive", "function_call", "shell_function"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::MAKE_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&hooks::MAKE_HOOKS)
    }
}
