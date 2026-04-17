//! HCL / Terraform language plugin.

pub mod connectors;
pub mod embedded;
pub mod primitives;
pub mod extract;
pub mod resolve;
pub(crate) mod builtins;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{EmbeddedRegion, ExtractionResult};

pub struct HclPlugin;

impl LanguagePlugin for HclPlugin {
    fn id(&self) -> &str {
        "hcl"
    }

    fn language_ids(&self) -> &[&str] {
        &["hcl", "terraform"]
    }

    fn extensions(&self) -> &[&str] {
        &[".hcl", ".tf", ".tfvars"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_hcl::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        extract::extract(source, tree_sitter_hcl::LANGUAGE.into())
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
            "block",
            "attribute",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "variable_expr",
            "get_attr",
            "function_call",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::HclResolver))
    }

    fn post_index(
        &self,
        db: &crate::db::Database,
        project_root: &std::path::Path,
        _ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        connectors::run_kubernetes(db, project_root);
    }
}
