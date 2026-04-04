//! Bicep (Azure IaC) language plugin.

pub mod extract;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct BicepPlugin;

impl LanguagePlugin for BicepPlugin {
    fn id(&self) -> &str {
        "bicep"
    }

    fn language_ids(&self) -> &[&str] {
        &["bicep"]
    }

    fn extensions(&self) -> &[&str] {
        &[".bicep"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_bicep::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let _ = source;
        ExtractionResult::empty()
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "resource_declaration",
            "module_declaration",
            "parameter_declaration",
            "variable_declaration",
            "output_declaration",
            "type_declaration",
            "user_defined_function",
            "metadata_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "import_statement",
            "import_with_statement",
            "import_functionality",
            "using_statement",
            "call_expression",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            // Bicep primitive types
            "string",
            "int",
            "bool",
            "object",
            "array",
            // Bicep parameterized types
            "resourceInput",
            "resourceOutput",
            // ARM common types (frequently referenced)
            "resource",
        ]
    }
}
