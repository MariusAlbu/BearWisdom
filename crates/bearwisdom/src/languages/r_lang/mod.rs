//! R language plugin.
//!
//! Grammar: tree-sitter-r (in Cargo.toml).
//! Extraction covers function assignments, S4/R6 class patterns, library imports, and calls.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

pub struct RLangPlugin;

impl LanguagePlugin for RLangPlugin {
    fn id(&self) -> &str { "r" }

    fn language_ids(&self) -> &[&str] { &["r"] }

    fn extensions(&self) -> &[&str] { &[".R", ".r", ".Rmd"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_r::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // R grammar uses `binary_operator` for ALL binary expressions, including
        // assignments (`<-`, `=`, `<<-`). Only assignment operators produce symbols;
        // ~60% of binary_operator nodes are assignments, which gives reasonable coverage.
        // `call` is excluded: only class/method/test calls produce symbols (~3% match rate),
        // which would drag the aggregate down without reflecting real extraction quality.
        &[
            "binary_operator",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
            "namespace_operator",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "numeric", "integer", "double", "complex", "character",
            "logical", "list", "vector", "matrix", "data.frame",
            "factor", "NULL", "NA", "TRUE", "FALSE", "Inf", "NaN",
        ]
    }
}
