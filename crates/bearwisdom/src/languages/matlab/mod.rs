//! MATLAB language plugin.
//!
//! Grammar: tree-sitter-matlab 1.3
//! What we extract:
//! - `function_definition` → Function
//! - `class_definition` → Class
//! - `assignment` (top-level) → Variable
//! - `arguments_statement` / `properties_block` (children of class) → handled via parent
//!
//! MATLAB files are typically one function or class per `.m` file.

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct MatlabPlugin;

impl LanguagePlugin for MatlabPlugin {
    fn id(&self) -> &str { "matlab" }

    fn language_ids(&self) -> &[&str] { &["matlab"] }

    fn extensions(&self) -> &[&str] { &[".m", ".mat"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_matlab::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_definition",
            "class_definition",
            "assignment",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call",
            "postfix_operator",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "double", "single", "int8", "int16", "int32", "int64",
            "uint8", "uint16", "uint32", "uint64",
            "char", "string", "logical", "cell", "struct", "table",
        ]
    }
}
