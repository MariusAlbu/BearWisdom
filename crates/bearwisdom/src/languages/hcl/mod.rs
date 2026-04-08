//! HCL / Terraform language plugin.

pub mod primitives;
pub mod extract;
pub mod resolve;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

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

    fn builtin_type_names(&self) -> &[&str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::HclResolver))
    }
}
