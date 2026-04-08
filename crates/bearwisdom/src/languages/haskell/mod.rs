//! Haskell language plugin.
//!
//! Grammar: tree-sitter-haskell (in Cargo.toml).
//! Extraction covers top-level functions, data/newtype, type classes, instances,
//! type synonyms, imports, and function-application calls.

mod builtins;
pub(crate) mod externals;
pub(crate) mod resolve;
pub mod primitives;
pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

pub struct HaskellPlugin;

impl LanguagePlugin for HaskellPlugin {
    fn id(&self) -> &str { "haskell" }

    fn language_ids(&self) -> &[&str] { &["haskell"] }

    fn extensions(&self) -> &[&str] { &[".hs", ".lhs"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_haskell::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::HASKELL_SCOPE_KINDS
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function",
            "data_type",
            "newtype",
            "class",
            "instance",
            "type_synomym",
            "foreign_import",
            "foreign_export",
            "pattern_synonym",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "import",
            "apply",
            "infix",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "Int", "Integer", "Float", "Double", "Bool", "Char", "String",
            "IO", "Maybe", "Either", "List", "Ordering", "Word",
            "Int8", "Int16", "Int32", "Int64",
            "Word8", "Word16", "Word32", "Word64",
            "Natural", "Rational", "Complex",
        ]
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn framework_globals(&self, dependencies: &std::collections::HashSet<String>) -> Vec<&'static str> {
        externals::framework_globals(dependencies)
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::HaskellResolver))
    }
}
