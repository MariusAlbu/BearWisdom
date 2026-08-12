//! Ada language plugin.
//!
//! Grammar: tree-sitter-ada 0.1
//! What we extract:
//! - `subprogram_declaration` → Function (specification only)
//! - `subprogram_body` → Function (with body)
//! - `package_declaration` → Namespace
//! - `package_body` → Namespace
//! - `full_type_declaration` → Struct or Enum
//! - `with_clause` → Imports edge

pub mod extract;
pub mod keywords;
mod predicates;
pub(crate) mod profile;
pub use profile::ADA_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct AdaPlugin;

impl LanguagePlugin for AdaPlugin {
    fn id(&self) -> &str {
        "ada"
    }

    fn language_ids(&self) -> &[&str] {
        &["ada"]
    }

    fn extensions(&self) -> &[&str] {
        &[".adb", ".ads"]
    }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_ada::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] {
        &[]
    }

    fn extract(&self, source: &str, _file_path: &str, _lang_id: &str) -> ExtractionResult {
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "subprogram_declaration",
            "subprogram_body",
            "subprogram_renaming_declaration",
            "generic_subprogram_declaration",
            "generic_package_declaration",
            "generic_instantiation",
            "package_declaration",
            "package_body",
            "full_type_declaration",
            "object_declaration",
            "parameter_specification",
            "component_declaration",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "with_clause",
            "use_clause",
            "use_type_clause",
            "procedure_call_statement",
            "function_call",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::ADA_PROFILE)
    }

}
