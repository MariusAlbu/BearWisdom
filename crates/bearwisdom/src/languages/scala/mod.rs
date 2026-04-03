//! scala language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
mod symbols;
pub mod extract;

mod builtins;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::ScalaResolver;

pub struct ScalaPlugin;

impl LanguagePlugin for ScalaPlugin {
    fn id(&self) -> &str { "scala" }

    fn language_ids(&self) -> &[&str] { &["scala"] }

    fn extensions(&self) -> &[&str] { &[".scala", ".sc"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_scala::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::SCALA_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }
}