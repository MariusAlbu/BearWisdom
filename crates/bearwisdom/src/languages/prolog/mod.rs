//! Prolog language plugin.
//!
//! Grammar: no tree-sitter grammar available on crates.io for Prolog.
//! Uses a line scanner that understands clause/fact/rule structure.

pub mod extract;
pub mod keywords;
mod predicates;
pub(crate) mod profile;
pub use profile::PROLOG_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct PrologPlugin;

impl LanguagePlugin for PrologPlugin {
    fn id(&self) -> &str {
        "prolog"
    }

    fn language_ids(&self) -> &[&str] {
        &["prolog"]
    }

    fn extensions(&self) -> &[&str] {
        &[".pl", ".pro", ".P"]
    }

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
        &["predicate_definition", "module_declaration", "use_module"]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["use_module", "goal"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PROLOG_PROFILE)
    }

}
