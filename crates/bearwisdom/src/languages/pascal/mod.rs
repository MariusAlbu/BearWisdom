//! Pascal / Delphi language plugin.
//!
//! Grammar: tree-sitter-pascal 0.10.2 — real grammar, LANGUAGE constant available.

pub mod keywords;
pub mod extract;
mod decls;
mod refs;
mod normalise;
mod error_recovery;

mod predicates;
pub(crate) mod hooks;
pub(crate) mod profile;
pub(crate) mod type_checker;
pub(crate) mod resolve;

pub use hooks::PASCAL_HOOKS;
pub use profile::PASCAL_PROFILE;

pub use resolve::PascalResolver;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PascalPlugin;

impl LanguagePlugin for PascalPlugin {
    fn id(&self) -> &str {
        "pascal"
    }

    fn language_ids(&self) -> &[&str] {
        &["pascal", "delphi"]
    }

    fn extensions(&self) -> &[&str] {
        &[".pas", ".pp", ".dpr", ".dpk", ".inc"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_pascal::LANGUAGE.into())
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
            "declProc",
            "defProc",
            "declClass",
            "declIntf",
            "declSection",
            "unit",
            "declUses",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "exprCall",
            "declUses",
            "typeref",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::PascalResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::PascalChecker))
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PASCAL_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks>
    {
        Some(&hooks::PASCAL_HOOKS)
    }
}
