//! TypeScript / TSX / JavaScript / JSX language plugin.
//!
//! Handles extraction for all four language IDs. The TypeScript and JavaScript
//! grammars are separate tree-sitter crates but share most extraction logic.
//! TSX and JSX use their respective grammars for JSX support.

// Extraction sub-modules
mod calls;
pub(crate) mod decorators;
mod helpers;
mod imports;
mod narrowing;
mod params;
mod symbols;
mod types;

pub mod extract;

// Resolution sub-modules
mod builtins;
mod chain;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

use crate::languages::LanguagePlugin;
use crate::types::ExtractionResult;
use crate::parser::scope_tree::ScopeKind;

// Re-export the resolver for registration in default_resolvers().
pub use resolve::TypeScriptResolver;

/// TypeScript language plugin — handles "typescript", "tsx", "javascript", "jsx".
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn id(&self) -> &str {
        "typescript"
    }

    fn language_ids(&self) -> &[&str] {
        &["typescript", "tsx"]
    }

    fn extensions(&self) -> &[&str] {
        &[".ts", ".tsx", ".mts", ".cts"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        Some(match lang_id {
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
            _ => return None,
        })
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::TS_SCOPE_KINDS
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let is_tsx = file_path.ends_with(".tsx") || lang_id == "tsx";
        extract::extract(source, is_tsx)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "class_declaration", "abstract_class_declaration",
            "interface_declaration",
            "function_declaration", "generator_function_declaration",
            "method_definition", "abstract_method_signature", "method_signature",
            "public_field_definition", "property_signature", "field_definition",
            "type_alias_declaration",
            "enum_declaration", "enum_body",
            "lexical_declaration", "variable_declaration",
            "internal_module",
            "construct_signature", "call_signature", "index_signature",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "call_expression",
            "new_expression",
            "import_statement",
            "jsx_self_closing_element", "jsx_opening_element",
            "extends_clause", "implements_clause",
            "type_annotation", "type_identifier",
            "as_expression", "satisfies_expression",
            "binary_expression", // instanceof
            "tagged_template_expression",
        ]
    }
}
