//! Prolog language plugin.
//!
//! Grammar: no tree-sitter grammar available on crates.io for Prolog.
//! Uses a line scanner that understands clause/fact/rule structure.

pub mod extract;

mod builtins;
pub(crate) mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PrologPlugin;

impl LanguagePlugin for PrologPlugin {
    fn id(&self) -> &str {
        "prolog"
    }

    fn language_ids(&self) -> &[&str] {
        &["prolog"]
    }

    fn extensions(&self) -> &[&str] {
        &[".pl", ".pro", ".P"]
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
            "predicate_definition",
            "module_declaration",
            "use_module",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "use_module",
            "goal",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PrologResolver))
    }
}
