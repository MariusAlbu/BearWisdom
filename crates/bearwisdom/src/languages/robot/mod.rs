//! Robot Framework language plugin.
//!
//! Grammar: no tree-sitter grammar in Cargo.toml.
//! `grammar()` returns `None`; extraction uses a line-oriented parser that
//! recognises Robot Framework's section-based structure.

pub mod primitives;
pub mod extract;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct RobotPlugin;

impl LanguagePlugin for RobotPlugin {
    fn id(&self) -> &str { "robot" }

    fn language_ids(&self) -> &[&str] { &["robot"] }

    fn extensions(&self) -> &[&str] { &[".robot"] }

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
            "keyword_definition",
            "test_case_definition",
            "variable_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "keyword_invocation",
            "setting_statement",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::RobotResolver))
    }
}
