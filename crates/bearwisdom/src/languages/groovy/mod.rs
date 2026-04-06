//! Groovy language plugin.
//!
//! Covers `.groovy` and `.gradle` files.
//!
//! What we extract:
//! - `class_declaration` → Class
//! - `function_definition` → Function (top-level `def`)
//! - `method_declaration` → Method (typed, inside class body)
//! - `package_declaration` → Namespace
//! - `import_declaration` → Imports
//! - `method_invocation` → Calls

pub(crate) mod primitives;
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
            "class_declaration",
            "function_definition",
            "method_declaration",
            "package_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "method_invocation",
            "import_declaration",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[
            "void", "boolean", "byte", "char", "short", "int", "long",
            "float", "double", "def", "String", "Object", "List", "Map",
            "GString", "BigDecimal", "BigInteger",
        ]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }
}
