//! Hare language plugin.
//!
//! Grammar: tree-sitter-hare v0.20.7 requires tree-sitter 0.20.6 (old ABI —
//! incompatible with tree-sitter 0.25). `grammar()` returns `None` until a
//! compatible release is published. Extraction is performed by a line-oriented
//! parser that recognises Hare's top-level declaration patterns.
//!
//! What we extract:
//! - `fn name(...)` → Function (export = Public)
//! - `type Name = struct/enum/...` → Struct/Enum/TypeAlias
//! - `def Name: type = value` → Variable (const)
//! - `let Name: type = value` → Variable (global)
//! - `use module::path;` → Imports edges
//! - `@test fn ...` → Test

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct HarePlugin;

impl LanguagePlugin for HarePlugin {
    fn id(&self) -> &str {
        "hare"
    }

    fn language_ids(&self) -> &[&str] {
        &["hare"]
    }

    fn extensions(&self) -> &[&str] {
        &[".ha"]
    }

    /// TODO: wire in tree_sitter_hare::language() once the crate is updated to
    /// tree-sitter 0.22+ (currently requires 0.20.6 — ABI-incompatible).
    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
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
            "function_declaration",
            "type_declaration",
            "const_declaration",
            "global_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "use_statement",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "bool", "void", "never",
            "int", "i8", "i16", "i32", "i64",
            "uint", "u8", "u16", "u32", "u64",
            "uintptr", "size",
            "f32", "f64",
            "rune", "str", "bytes",
            "null",
        ]
    }
}
