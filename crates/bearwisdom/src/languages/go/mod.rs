//! go language plugin.

mod calls;
mod helpers;
mod symbols;
mod tags;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::parser::extractors::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::GoResolver;

pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn id(&self) -> &str { "go" }

    fn language_ids(&self) -> &[&str] { &["go"] }

    fn extensions(&self) -> &[&str] { &[".go"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_go::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }
}