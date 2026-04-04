//! Zig language plugin.
//!
//! `grammar()` returns the tree-sitter-zig grammar; extraction is also performed by a line-oriented
//! parser that recognises Zig's top-level declaration patterns.
//!
//! What we extract:
//! - `fn name(...)` → Function (with pub/export visibility)
//! - `const Name = struct { ... }` → Struct
//! - `const Name = enum { ... }` → Enum
//! - `const Name = union { ... }` → Struct (tagged union)
//! - `const Name = error { ... }` → Enum (error set)
//! - `const/var name` (plain) → Variable
//! - `test "name" { ... }` → Test
//! - `@import("path")` assignments → Imports edges
//! - function calls in the body → Calls edges (best-effort identifier scan)

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct ZigPlugin;

impl LanguagePlugin for ZigPlugin {
    fn id(&self) -> &str {
        "zig"
    }

    fn language_ids(&self) -> &[&str] {
        &["zig"]
    }

    fn extensions(&self) -> &[&str] {
        &[".zig"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_zig::LANGUAGE.into())
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
            "variable_declaration",
            "test_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["call_expression", "builtin_function"]
    }

    fn builtin_type_names(&self) -> &[&str] {
        // Zig primitive types
        &[
            "bool", "void", "noreturn", "type", "anyerror", "anyframe", "anytype",
            "comptime_int", "comptime_float",
            "i8", "i16", "i32", "i64", "i128", "isize",
            "u8", "u16", "u32", "u64", "u128", "usize",
            "f16", "f32", "f64", "f80", "f128",
            "c_short", "c_int", "c_long", "c_longlong",
            "c_ushort", "c_uint", "c_ulong", "c_ulonglong",
            "c_char", "c_longdouble",
        ]
    }
}
