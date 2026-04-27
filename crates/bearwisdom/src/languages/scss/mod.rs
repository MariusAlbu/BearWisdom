//! SCSS language plugin.
//!
//! Uses the dedicated SCSS tree-sitter grammar (tree-sitter-scss-local),
//! compiled from MSVC-compatible pre-expanded C source.

pub(crate) mod predicates;
pub(crate) mod type_checker;
pub mod keywords;
pub mod extract;
pub mod resolve;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

pub struct ScssPlugin;

impl LanguagePlugin for ScssPlugin {
    fn id(&self) -> &str {
        "scss"
    }

    fn language_ids(&self) -> &[&str] {
        &["scss", "sass"]
    }

    fn extensions(&self) -> &[&str] {
        &[".scss", ".sass"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_scss_local::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source, file_path)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "mixin_statement",
            "function_statement",
            "keyframes_statement",
            "rule_set",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "include_statement",
            "extend_statement",
            "import_statement",
            "forward_statement",
            "call_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::ScssResolver))
    }


    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::ScssChecker))
    }
}
