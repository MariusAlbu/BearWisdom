//! bash language plugin.

pub mod keywords;
pub mod extract;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;
pub(crate) mod type_checker;
pub(crate) mod resolve;

pub use hooks::BASH_HOOKS;
pub use profile::BASH_PROFILE;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub struct BashPlugin;

impl LanguagePlugin for BashPlugin {
    // The plugin's directory is `bash`, but the language tag the registry
    // uses is `"shell"` so SCM grammar names and other downstream code stay
    // language-neutral across `.sh`/`.bash`/`.zsh`. `id()` must agree with
    // `language_ids()` so `language_by_extension()` (which falls back to
    // `id()`) produces a tag the registry's `by_lang_id` actually maps —
    // returning `"bash"` would route every shell file to the generic
    // fallback plugin and emit zero symbols. Same shape as the rust_lang
    // / c_lang fixes (PR 104, PR 109).
    fn id(&self) -> &str { "shell" }

    fn language_ids(&self) -> &[&str] { &["shell"] }

    fn extensions(&self) -> &[&str] { &[".sh", ".bash", ".zsh"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_bash::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_definition",
            // declaration_command listed before variable_assignment so the
            // coverage correlator matches declaration lines to declaration_command
            // first (the child variable_assignment shares the same line).
            "declaration_command",
            "variable_assignment",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            // command_substitution listed before command so the coverage
            // correlator claims the substitution node when both a command and
            // its enclosing substitution share the same line.
            "command_substitution",
            "command",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::BASH_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::BASH_HOOKS)
    }
}