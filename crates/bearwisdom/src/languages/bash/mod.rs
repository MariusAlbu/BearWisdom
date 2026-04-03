//! bash language plugin.

pub mod extract;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

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
            "variable_assignment",
            "declaration_command",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "command",
            "command_substitution",
        ]
    }
}