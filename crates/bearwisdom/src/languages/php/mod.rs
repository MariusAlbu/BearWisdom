//! php language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod symbols;
pub mod extract;

mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::PhpResolver;

pub struct PhpPlugin;

impl LanguagePlugin for PhpPlugin {
    fn id(&self) -> &str { "php" }

    fn language_ids(&self) -> &[&str] { &["php"] }

    fn extensions(&self) -> &[&str] { &[".php"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_php::LANGUAGE_PHP.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::PHP_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }
}