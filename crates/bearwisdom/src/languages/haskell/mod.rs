//! Haskell language plugin.
//!
//! Grammar: tree-sitter-haskell (in Cargo.toml).
//! Extraction covers top-level functions, data/newtype, type classes, instances,
//! type synonyms, imports, and function-application calls.

mod predicates;
pub(crate) mod type_checker;
pub(crate) mod resolve;
pub mod keywords;
pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "probe_test.rs"]
mod probe_test;

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

    fn keywords(&self) -> &'static [&'static str] {
        &[
            "Int", "Integer", "Float", "Double", "Bool", "Char", "String",
            "IO", "Maybe", "Either", "List", "Ordering", "Word",
            "Int8", "Int16", "Int32", "Int64",
            "Word8", "Word16", "Word32", "Word64",
            "Natural", "Rational", "Complex",
        ]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::HaskellResolver))
    }


    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::HaskellChecker))
    }
}
