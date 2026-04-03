//! javascript language plugin.

mod helpers;
pub mod extract;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub struct JavascriptPlugin;

impl LanguagePlugin for JavascriptPlugin {
    fn id(&self) -> &str { "javascript" }

    fn language_ids(&self) -> &[&str] { &["javascript", "jsx"] }

    fn extensions(&self) -> &[&str] { &[".js", ".jsx", ".mjs", ".cjs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_javascript::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }
}