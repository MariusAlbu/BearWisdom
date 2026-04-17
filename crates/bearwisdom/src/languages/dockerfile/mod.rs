//! Dockerfile language plugin.

mod predicates;
pub mod connectors;
pub mod keywords;
pub mod embedded;
pub mod extract;
pub mod resolve;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

pub struct DockerfilePlugin;

impl LanguagePlugin for DockerfilePlugin {
    fn id(&self) -> &str {
        "dockerfile"
    }

    fn language_ids(&self) -> &[&str] {
        &["dockerfile"]
    }

    /// No file extensions — Dockerfile detection is handled by bearwisdom-profile
    /// via filename matching ("Dockerfile", "*.dockerfile", etc.).
    fn extensions(&self) -> &[&str] {
        &[]
    }

    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {
        let _ = lang_id;
        Some(tree_sitter_dockerfile_0_25::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source)
    }

    fn embedded_regions(&self, source: &str, _p: &str, _l: &str) -> Vec<EmbeddedRegion> {
        embedded::detect_regions(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "from_instruction",
            "arg_instruction",
            "env_instruction",
            "label_instruction",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "copy_instruction",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::DockerfileResolver))
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        connectors::run_docker_compose(db, project_root);
    }
}
