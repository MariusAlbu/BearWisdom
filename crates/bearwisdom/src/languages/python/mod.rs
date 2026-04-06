//! python language plugin.

mod calls;
pub(crate) mod decorators;
mod helpers;
pub(crate) mod primitives;
mod symbols;
pub mod extract;

mod builtins;
mod chain;
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

pub use resolve::PythonResolver;

pub struct PythonPlugin;

impl LanguagePlugin for PythonPlugin {
    fn id(&self) -> &str { "python" }

    fn language_ids(&self) -> &[&str] { &["python"] }

    fn extensions(&self) -> &[&str] { &[".py", ".pyi"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_python::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "function_definition",
            // `decorated_definition` wraps class_definition/function_definition;
            // those inner node kinds already cover decorated defs when the
            // start_line is not patched to the decorator line.
            "assignment",
            "type_alias_statement",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call",
            "import_statement",
            "import_from_statement",
            "future_import_statement",
            "typed_parameter",
            "typed_default_parameter",
            "type",
            "generic_type",
            "union_type",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &["int", "float", "str", "bool", "bytes", "None", "list", "dict", "set", "tuple", "type", "object", "complex"]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }
}