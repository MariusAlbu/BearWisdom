//! Haskell language plugin.
//!
//! Grammar: tree-sitter-haskell (in Cargo.toml).
//! Extraction covers top-level functions, data/newtype, type classes, instances,
//! type synonyms, imports, and function-application calls.

pub mod extract;
pub(crate) mod hooks;
pub mod keywords;

#[cfg(test)]
#[path = "keywords_tests.rs"]
mod keywords_tests;

mod predicates;
pub(crate) mod profile;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

pub use hooks::HASKELL_HOOKS;
pub use profile::HASKELL_PROFILE;
mod definitions;
mod expressions;
mod servant;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "probe_test.rs"]
mod probe_test;

pub struct HaskellPlugin;

impl LanguagePlugin for HaskellPlugin {
    fn id(&self) -> &str {
        "haskell"
    }

    fn language_ids(&self) -> &[&str] {
        &["haskell"]
    }

    fn extensions(&self) -> &[&str] {
        &[".hs", ".lhs"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_haskell::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        extract::HASKELL_SCOPE_KINDS
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "function",
            "data_type",
            "newtype",
            "class",
            "instance",
            "type_synomym",
            "foreign_import",
            "foreign_export",
            "pattern_synonym",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["import", "apply", "infix"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::HASKELL_PROFILE)
    }

    fn language_hooks(
        &self,
    ) -> Option<&'static dyn crate::type_checker::profile::hooks::LanguageEngineHooks> {
        Some(&hooks::HASKELL_HOOKS)
    }
}
