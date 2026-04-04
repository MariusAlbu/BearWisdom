//! VBA (Visual Basic for Applications) language plugin.
//!
//! Grammar: no tree-sitter grammar available on crates.io.
//! Uses a line scanner over VBA source.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct VbaPlugin;

impl LanguagePlugin for VbaPlugin {
    fn id(&self) -> &str {
        "vba"
    }

    fn language_ids(&self) -> &[&str] {
        &["vba"]
    }

    fn extensions(&self) -> &[&str] {
        &[".bas", ".cls", ".frm"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        None
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "sub_declaration",
            "function_declaration",
            "class_module",
            "property_declaration",
            "variable_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_statement",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "Integer",
            "Long",
            "LongLong",
            "Single",
            "Double",
            "Currency",
            "Decimal",
            "Boolean",
            "Byte",
            "Date",
            "String",
            "Object",
            "Variant",
            "Nothing",
            "Empty",
            "Null",
        ]
    }
}
