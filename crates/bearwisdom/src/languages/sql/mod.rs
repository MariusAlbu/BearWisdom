//! SQL language plugin.

pub(crate) mod primitives;
pub mod extract;
pub mod resolve;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

pub struct SqlPlugin;

impl LanguagePlugin for SqlPlugin {
    fn id(&self) -> &str {
        "sql"
    }

    fn language_ids(&self) -> &[&str] {
        &["sql"]
    }

    fn extensions(&self) -> &[&str] {
        &[".sql"]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_sequel::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        // Actual node kinds produced by tree-sitter-sequel 0.3.x
        &[
            "create_table",      // CREATE TABLE → SymbolKind::Struct
            "create_view",       // CREATE VIEW  → SymbolKind::Class
            "create_index",      // CREATE INDEX → SymbolKind::Variable
            "create_function",   // CREATE FUNCTION → SymbolKind::Function
            "column_definition", // column inside a table → SymbolKind::Field
            "cte",               // WITH … AS (…) → SymbolKind::Variable (if desired)
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        // tree-sitter-sequel uses object_reference for every name reference.
        // FK inline REFERENCES lives as object_reference directly under column_definition.
        // ALTER TABLE target is an object_reference under alter_table.
        &[
            "object_reference", // covers FK REFERENCES, ALTER TABLE target, view FROM targets
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn primitives(&self) -> &'static [&'static str] {
        primitives::PRIMITIVES
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::SqlResolver))
    }
}
