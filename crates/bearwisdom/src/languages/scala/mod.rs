//! scala language plugin.

mod calls;
pub(crate) mod decorators;
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

pub use resolve::ScalaResolver;

pub struct ScalaPlugin;

impl LanguagePlugin for ScalaPlugin {
    fn id(&self) -> &str { "scala" }

    fn language_ids(&self) -> &[&str] { &["scala"] }

    fn extensions(&self) -> &[&str] { &[".scala", ".sc"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_scala::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { extract::SCALA_SCOPE_KINDS }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_definition",
            "object_definition",
            "trait_definition",
            "enum_definition",
            "full_enum_case",
            "simple_enum_case",
            "function_definition",
            "function_declaration",
            "val_definition",
            "var_definition",
            "val_declaration",
            "var_declaration",
            "type_definition",
            "given_definition",
            "package_clause",
            "package_object",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "instance_expression",
            "import_declaration",
            "export_declaration",
            "type_identifier",
            "type_arguments",
            "extends_clause",
            "infix_expression",
        ]
    }
}