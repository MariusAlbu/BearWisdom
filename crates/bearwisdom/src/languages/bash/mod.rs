//! bash language plugin.

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod resolve;

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
    fn id(&self) -> &str { "bash" }

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

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::BashResolver))
    }
}