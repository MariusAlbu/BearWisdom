//! F# language plugin.
//!
//! Covers `.fs`, `.fsi`, `.fsx` files.
//!
//! Uses `LANGUAGE_FSHARP` constant (tree-sitter-fsharp exposes two grammars:
//! `LANGUAGE_FSHARP` for `.fs`/`.fsx` and `LANGUAGE_SIGNATURE` for `.fsi`).
//! We use `LANGUAGE_FSHARP` for all variants.
//!
//! What we extract:
//! - `function_or_value_defn` → Function / Variable
//! - `type_definition` → Class / Struct / Enum / Interface / TypeAlias
//! - `module_defn` / `named_module` / `namespace` → Namespace
//! - `import_decl` → Imports (open declarations)

pub(crate) mod primitives;
mod builtins;
pub(crate) mod resolve;
pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct FSharpPlugin;

impl LanguagePlugin for FSharpPlugin {
    fn id(&self) -> &str { "fsharp" }

    fn language_ids(&self) -> &[&str] { &["fsharp"] }

    fn extensions(&self) -> &[&str] { &[".fs", ".fsi", ".fsx"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_fsharp::LANGUAGE_FSHARP.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_or_value_defn",
            "type_definition",
            "module_defn",
            "named_module",
            "namespace",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "application_expression",
            "dot_expression",
            "import_decl",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "int", "int8", "int16", "int32", "int64",
            "uint8", "uint16", "uint32", "uint64",
            "float", "float32", "double", "decimal",
            "bool", "char", "string", "unit", "obj", "byte",
            "sbyte", "nativeint", "unativeint", "bigint",
            "exn", "void",
        ]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::FSharpResolver))
    }
}
