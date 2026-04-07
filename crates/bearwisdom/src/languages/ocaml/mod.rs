//! OCaml language plugin.
//!
//! Grammar: tree-sitter-ocaml 0.24 — uses `LANGUAGE_OCAML` for `.ml`,
//! `LANGUAGE_OCAML_INTERFACE` for `.mli`.
//!
//! What we extract:
//! - `value_definition` → Function or Variable
//! - `type_definition` → TypeAlias / Struct / Enum
//! - `module_definition` → Namespace
//! - `open_module` → Imports edge

mod builtins;
pub(crate) mod resolve;
pub mod primitives;
pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct OcamlPlugin;

impl LanguagePlugin for OcamlPlugin {
    fn id(&self) -> &str { "ocaml" }

    fn language_ids(&self) -> &[&str] { &["ocaml"] }

    fn extensions(&self) -> &[&str] { &[".ml", ".mli"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        match lang_id {
            "ocaml" => Some(tree_sitter_ocaml::LANGUAGE_OCAML.into()),
            _ => Some(tree_sitter_ocaml::LANGUAGE_OCAML.into()),
        }
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "value_definition",
            "type_definition",
            "module_definition",
            "open_module",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "open_module",
            "application_expression",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "int", "float", "bool", "char", "string", "unit",
            "bytes", "exn", "format",
            "list", "array", "option", "result",
            "int32", "int64", "nativeint",
        ]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::OcamlResolver))
    }
}
