//! rust_lang language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod patterns;
mod symbols;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::RustResolver;

pub struct RustLangPlugin;

impl LanguagePlugin for RustLangPlugin {
    fn id(&self) -> &str { "rust_lang" }

    fn language_ids(&self) -> &[&str] { &["rust"] }

    fn extensions(&self) -> &[&str] { &[".rs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_rust::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }
}