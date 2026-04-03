//! Starlark / Bazel BUILD file language plugin.
//!
//! Grammar: no tree-sitter grammar in Cargo.toml.
//! `grammar()` returns `None`; extraction uses a line-oriented parser that
//! recognises Starlark's `def`, `load()`, rule assignments, and calls.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct StarlarkPlugin;

impl LanguagePlugin for StarlarkPlugin {
    fn id(&self) -> &str { "starlark" }

    fn language_ids(&self) -> &[&str] { &["starlark"] }

    fn extensions(&self) -> &[&str] { &[".bzl", ".star"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
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

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
