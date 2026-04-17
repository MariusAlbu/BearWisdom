//! Zig language plugin.
//!
//! Uses a line-oriented parser (no grammar dependency at extraction time) that
//! recognises Zig's declaration patterns.
//!
//! What we extract:
//! - `fn name(...)` → Function (top-level, with pub/export visibility)
//! - Methods inside `const Name = struct/union/enum { ... }` → Method
//! - Methods inside `return struct { ... }` (comptime generic types) → Method
//! - `const Name = struct { ... }` → Struct
//! - `const Name = enum { ... }` → Enum
//! - `const Name = union { ... }` → Struct (tagged union)
//! - `const Name = error { ... }` → Enum (error set)
//! - `const/var name` (plain) → Variable
//! - `test "name" { ... }` → Test
//! - `@import("path")` assignments → Imports edges
//! - `identifier(` patterns in bodies → Calls edges
//! - `@builtin(` patterns (everywhere) → Calls edges

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod resolve;

pub use resolve::ZigResolver;

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
        // variable_declaration in tree-sitter-zig applies to both top-level and
        // local declarations — including every `const`/`var` inside function bodies.
        // The extractor intentionally skips local variables (they are noise), so
        // including variable_declaration in coverage rules would inflate the
        // denominator with thousands of local variables that are never extracted.
        &[
            "function_declaration",
            "test_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["call_expression", "builtin_function"]
    }

    fn keywords(&self) -> &'static [&'static str] {
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

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::ZigResolver))
    }

}
