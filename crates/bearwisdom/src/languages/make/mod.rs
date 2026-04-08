//! Make / Makefile language plugin.

pub mod primitives;
pub mod extract;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct MakePlugin;

impl LanguagePlugin for MakePlugin {
    fn id(&self) -> &str {
        "make"
    }

    fn language_ids(&self) -> &[&str] {
        &["make"]
    }

    /// Extensions for Make files. `Makefile` (no dot) is detected by
    /// bearwisdom-profile via filename matching.
    fn extensions(&self) -> &[&str] {
        &["Makefile", ".mk"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_make::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let _ = source;
        ExtractionResult::empty()
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "rule",
            "variable_assignment",
            "define_directive",
            "shell_assignment",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "include_directive",
            "function_call",
            "shell_function",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::MakeResolver))
    }
}
