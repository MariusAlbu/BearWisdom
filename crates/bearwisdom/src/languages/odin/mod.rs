//! Odin language plugin.
//!
//! `grammar()` returns the tree-sitter-odin grammar; extraction is also performed by a line-oriented
//! parser that recognises Odin's top-level declaration patterns.
//!
//! What we extract:
//! - `name :: proc(...)` → Function
//! - `name :: struct { ... }` → Struct
//! - `name :: enum { ... }` → Enum
//! - `name :: union { ... }` → Struct (tagged union)
//! - `name :: value` / `name: Type = value` → Variable
//! - `import "path"` / `import name "path"` → Imports edges
//! - `using expr` → TypeRef edge

pub mod primitives;
pub mod extract;

mod builtins;
pub(crate) mod externals;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct OdinPlugin;

impl LanguagePlugin for OdinPlugin {
    fn id(&self) -> &str {
        "odin"
    }

    fn language_ids(&self) -> &[&str] {
        &["odin"]
    }

    fn extensions(&self) -> &[&str] {
        &[".odin"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_odin::LANGUAGE.into())
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
            "procedure_declaration",
            "struct_declaration",
            "enum_declaration",
            "union_declaration",
            "import_declaration",
            "overloaded_procedure_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "using_statement",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "bool", "b8", "b16", "b32", "b64",
            "int", "i8", "i16", "i32", "i64", "i128",
            "uint", "u8", "u16", "u32", "u64", "u128",
            "uintptr", "rawptr",
            "f16", "f32", "f64",
            "complex32", "complex64", "complex128",
            "quaternion64", "quaternion128", "quaternion256",
            "string", "cstring", "rune", "byte",
            "typeid", "any", "void",
        ]
    }

    fn externals(&self) -> &'static [&'static str] {
        externals::EXTERNALS
    }

    fn framework_globals(&self, dependencies: &std::collections::HashSet<String>) -> Vec<&'static str> {
        externals::framework_globals(dependencies)
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::OdinResolver))
    }
}
