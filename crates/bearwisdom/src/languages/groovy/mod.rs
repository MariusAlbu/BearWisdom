//! Groovy language plugin.
//!
//! Covers `.groovy` and `.gradle` files.
//!
//! What we extract:
//! - `class_definition` → Class
//! - `function_definition` → Function / Method
//! - `groovy_package` → Namespace
//! - `declaration` (module-level) → Variable
//! - `groovy_import` → Imports
//! - `function_call` / `juxt_function_call` → Calls

pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct GroovyPlugin;

impl LanguagePlugin for GroovyPlugin {
    fn id(&self) -> &str { "groovy" }

    fn language_ids(&self) -> &[&str] { &["groovy"] }

    fn extensions(&self) -> &[&str] { &[".groovy", ".gradle"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_groovy::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "function_definition",
            "function_declaration",
            "groovy_package",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "function_call",
            "juxt_function_call",
            "groovy_import",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "void", "boolean", "byte", "char", "short", "int", "long",
            "float", "double", "def", "String", "Object", "List", "Map",
            "GString", "BigDecimal", "BigInteger",
        ]
    }
}
