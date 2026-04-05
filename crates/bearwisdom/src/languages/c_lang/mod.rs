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

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

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
        // Only register node kinds where extraction reliably covers >= 95% of
        // all occurrences.
        //
        // Excluded from tracking:
        //   struct_specifier / union_specifier / enum_specifier — these nodes
        //   appear both as DEFINITIONS (struct Foo { ... }) and as TYPE
        //   REFERENCES (struct Foo *ptr). We only emit symbols for definitions;
        //   the type-reference occurrences inflate the denominator and make 95%
        //   coverage structurally impossible to achieve.
        //   field_declaration — nested field declarations inside anonymous or
        //   complex struct hierarchies are not always reachable.
        //   type_definition — typedef bodies may contain unnamed specifiers whose
        //   inner symbol is emitted at a different line from the typedef wrapper.
        //
        // NOTE: template_declaration is intentionally excluded — the extractor
        // emits the symbol for the inner node (function_definition, class_specifier,
        // etc.) which is already tracked via those kinds.
        &[
            "function_definition",
            "declaration",
            "enumerator",
            "preproc_def",
            "preproc_function_def",
            // C++ additions
            "class_specifier",
            "namespace_definition",
            "namespace_alias_definition",
            "alias_declaration",
            "concept_definition",
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

    fn builtin_type_names(&self) -> &[&str] {
        &["int", "char", "void", "float", "double", "short", "long", "unsigned", "signed", "size_t", "bool", "auto"]
    }
}