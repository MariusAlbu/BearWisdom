//! Gleam language plugin.
//!
//! `grammar()` returns the tree-sitter-gleam grammar; extraction is also performed by a line-oriented
//! parser that recognises Gleam's top-level declaration patterns.
//!
//! What we extract:
//! - `pub fn name(...)` / `fn name(...)` → Function (pub = Public)
//! - `pub type Name { ... }` / `type Name { ... }` → Enum (ADT/custom type)
//! - `pub type Name = OtherType` → TypeAlias
//! - `@external(erlang, ...) pub fn name(...)` → Function (FFI)
//! - `import module` / `import module.{symbol}` → Imports edges
//! - `pub const name = ...` / `const name = ...` → Variable
//! - `value |> func(...)` pipelines → Calls edges

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GleamPlugin;

impl LanguagePlugin for GleamPlugin {
    fn id(&self) -> &str {
        "gleam"
    }

    fn language_ids(&self) -> &[&str] {
        &["gleam"]
    }

    fn extensions(&self) -> &[&str] {
        &[".gleam"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_gleam::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        let _ = file_path;
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function",
            "external_function",
            "type_definition",
            "type_alias",
            "constant",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call",
            "binary_expression",
            "import",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "Int", "Float", "Bool", "String", "BitArray",
            "List", "Result", "Option", "Nil",
            "Dynamic", "UtfCodepoint",
        ]
    }
}
