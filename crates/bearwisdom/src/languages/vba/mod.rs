//! VBA (Visual Basic for Applications) language plugin.
//!
//! Grammar: no tree-sitter grammar available on crates.io.
//! Uses a line scanner over VBA source.

pub mod extract;

mod keywords;
mod predicates;
pub(crate) mod type_checker;
pub(crate) mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

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

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::VbaResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::VbaChecker))
    }
}
