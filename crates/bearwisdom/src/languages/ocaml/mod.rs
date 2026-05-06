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

mod predicates;
pub(crate) mod type_checker;
pub(crate) mod resolve;
pub mod keywords;
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

    fn keywords(&self) -> &'static [&'static str] {
        &[
            // Primitive types
            "int", "float", "bool", "char", "string", "unit",
            "bytes", "exn", "format",
            "list", "array", "option", "result",
            "int32", "int64", "nativeint",
            // Stdlib I/O
            "print_string", "print_endline", "print_int", "print_float",
            "print_char", "print_newline",
            "prerr_string", "prerr_endline",
            "read_line", "read_int", "read_float",
            "input_line", "output_string",
            // Conversion
            "string_of_int", "int_of_string",
            "string_of_float", "float_of_string",
            "string_of_bool", "bool_of_string",
            "char_of_int", "int_of_char",
            "float_of_int", "int_of_float",
            // Control / pair
            "ignore", "failwith", "invalid_arg", "raise", "assert",
            "fst", "snd",
            // Numeric primitives
            "min", "max", "abs", "succ", "pred", "mod_float",
            "sqrt", "exp", "log", "log10",
            "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
            "ceil", "floor", "truncate",
            // References
            "ref", "incr", "decr", "not", "compare",
            // Stdlib modules (always-in-scope roots)
            "List", "Array", "String", "Bytes", "Buffer",
            "Hashtbl", "Map", "Set", "Stack", "Queue", "Stream",
            "Scanf", "Printf", "Format",
            "Sys", "Unix", "Filename", "Arg", "Printexc",
            "Lazy", "Fun", "Seq", "Option", "Result", "Either",
            // Constructors
            "Some", "None", "Ok", "Error",
            // Literals
            "true", "false",
        ]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::OcamlResolver))
    }


    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::OCamlChecker))
    }
}
