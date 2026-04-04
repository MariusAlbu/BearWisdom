//! SQL language plugin.

pub mod extract;

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
        &[
            "create_table_stmt",
            "create_view_stmt",
            "create_trigger_stmt",
            "column_def",
            "common_table_expression",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "table_or_subquery",
            "foreign_key_clause",
            "type_name",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }
}
