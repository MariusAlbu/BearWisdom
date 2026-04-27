//! Nix language plugin.

pub mod keywords;
pub mod extract;
pub mod resolve;
pub(crate) mod predicates;
pub(crate) mod type_checker;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct NixPlugin;

impl LanguagePlugin for NixPlugin {
    fn id(&self) -> &str {
        "nix"
    }

    fn language_ids(&self) -> &[&str] {
        &["nix"]
    }

    fn extensions(&self) -> &[&str] {
        &[".nix"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_nix::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {
        let _ = (file_path, lang_id);
        let language: tree_sitter::Language = tree_sitter_nix::LANGUAGE.into();
        extract::extract(source, language)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "binding",
            "inherit",
            "inherit_from",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "apply_expression",
            "with_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    /// In Nix, curried application (`f a b`) parses as two nested
    /// `apply_expression` nodes. The extractor emits one ref per call site
    /// (the outermost application), so inner apply nodes must not be counted
    /// in the coverage denominator.
    fn nested_ref_skip_pairs(&self) -> &[(&'static str, &'static str)] {
        &[("apply_expression", "apply_expression")]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::NixResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::NixChecker))
    }
}
