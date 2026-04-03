//! c_lang language plugin.

mod calls;
mod helpers;
mod symbols;
pub mod extract;

mod builtins;
pub mod resolve;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub use resolve::CLangResolver;

pub struct CLangPlugin;

impl LanguagePlugin for CLangPlugin {
    fn id(&self) -> &str { "c_lang" }

    fn language_ids(&self) -> &[&str] { &["c", "cpp"] }

    fn extensions(&self) -> &[&str] { &[".c", ".h", ".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        match lang_id {
            "c" => Some(tree_sitter_c::LANGUAGE.into()),
            "cpp" => Some(tree_sitter_cpp::LANGUAGE.into()),
            _ => None,
        }
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::C_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        extract::extract(source, lang_id)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Shared C and C++ node kinds; C++ adds class_specifier, namespace_definition,
        // alias_declaration, concept_definition on top.
        &[
            "function_definition",
            "declaration",
            "struct_specifier",
            "union_specifier",
            "enum_specifier",
            "enumerator",
            "field_declaration",
            "type_definition",
            "preproc_def",
            "preproc_function_def",
            // C++ additions
            "class_specifier",
            "namespace_definition",
            "namespace_alias_definition",
            "alias_declaration",
            "concept_definition",
            "template_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "preproc_include",
            "type_identifier",
            "base_class_clause",
            "cast_expression",
            "sizeof_expression",
            "template_argument_list",
            // C++ import (C++20 modules)
            "import_declaration",
        ]
    }
}