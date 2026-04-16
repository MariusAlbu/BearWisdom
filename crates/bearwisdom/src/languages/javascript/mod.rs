//! javascript language plugin.

mod helpers;
pub(crate) mod builtins;
pub(crate) mod primitives;
pub mod extract;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

pub struct JavascriptPlugin;

impl LanguagePlugin for JavascriptPlugin {
    fn id(&self) -> &str { "javascript" }

    fn language_ids(&self) -> &[&str] { &["javascript", "jsx"] }

    fn extensions(&self) -> &[&str] { &[".js", ".jsx", ".mjs", ".cjs"] }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_javascript::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration",
            "class",
            "function_declaration",
            "generator_function_declaration",
            // `function_expression` is intentionally omitted: standalone function
            // expressions (as callbacks, IIFEs, object property values) have no
            // extractable name. Named cases like `const f = function() {}` are
            // already captured under the parent `lexical_declaration` node.
            "method_definition",
            "variable_declaration",
            "lexical_declaration",
            "field_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "import_statement",
            "export_statement",
            "class_heritage",
            "jsx_opening_element",
            "jsx_self_closing_element",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(crate::languages::typescript::resolve::TypeScriptResolver))
    }

}