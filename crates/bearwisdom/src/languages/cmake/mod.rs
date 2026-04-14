//! CMake language plugin.

pub mod embedded;
pub mod primitives;
pub mod extract;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct CMakePlugin;

impl LanguagePlugin for CMakePlugin {
    fn id(&self) -> &str {
        "cmake"
    }

    fn language_ids(&self) -> &[&str] {
        &["cmake"]
    }

    /// `.cmake` extension. `CMakeLists.txt` is detected by bearwisdom-profile
    /// via filename matching.
    fn extensions(&self) -> &[&str] {
        &[".cmake"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_cmake::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source, tree_sitter_cmake::LANGUAGE.into())
    }

    fn embedded_regions(
        &self,
        source: &str,
        _file_path: &str,
        _lang_id: &str,
    ) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function_def",
            "macro_def",
            "normal_command",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "normal_command",
            "variable_ref",
        ]
    }

    fn builtin_type_names(&self) -> &[&str] {
        // CMake has no type system; no builtins to exclude.
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::CMakeResolver))
    }
}
