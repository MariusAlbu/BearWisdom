//! elixir language plugin.

mod helpers;
pub mod extract;

mod builtins;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::ElixirResolver;

pub struct ElixirPlugin;

impl LanguagePlugin for ElixirPlugin {
    fn id(&self) -> &str { "elixir" }

    fn language_ids(&self) -> &[&str] { &["elixir"] }

    fn extensions(&self) -> &[&str] { &[".ex", ".exs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_elixir::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Elixir's grammar represents all constructs as `call` nodes.
        // The extractor dispatches on the callee name (defmodule, def, defp, etc.).
        // Coverage correlates by line — listing "call" lets the metric count the
        // fraction of call nodes that produce a symbol extraction.
        &["call"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "dot",
            "alias",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}