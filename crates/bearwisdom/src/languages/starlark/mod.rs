//! Starlark / Bazel BUILD file language plugin.
//!
//! `grammar()` returns the tree-sitter-starlark grammar; extraction also uses a line-oriented parser that
//! recognises Starlark's `def`, `load()`, rule assignments, and calls.

pub mod embedded;
pub mod keywords;
pub mod extract;
pub mod resolve;
pub(crate) mod chain;
mod predicates;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct StarlarkPlugin;

impl LanguagePlugin for StarlarkPlugin {
    fn id(&self) -> &str { "starlark" }

    fn language_ids(&self) -> &[&str] { &["starlark"] }

    fn extensions(&self) -> &[&str] { &[".bzl", ".star"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_starlark::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_definition",
            "assignment",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::StarlarkResolver))
    }
}
