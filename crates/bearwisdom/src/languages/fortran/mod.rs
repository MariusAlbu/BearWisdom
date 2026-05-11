//! Fortran language plugin.
//!
//! Grammar: tree-sitter-fortran 0.5
//! What we extract:
//! - `subroutine` (via `subroutine_statement` head) → Function
//! - `function` (via `function_statement` head) → Function
//! - `module` (via `module_statement`) → Namespace
//! - `derived_type_definition` → Struct
//! - `use_statement` → Imports edge

pub mod keywords;
pub mod extract;
pub mod fypp;

mod predicates;
pub(crate) mod type_checker;
pub(crate) mod resolve;

pub use resolve::FortranResolver;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::ExtractionResult;

pub struct FortranPlugin;

impl LanguagePlugin for FortranPlugin {
    fn id(&self) -> &str { "fortran" }

    fn language_ids(&self) -> &[&str] { &["fortran"] }

    fn extensions(&self) -> &[&str] { &[".f90", ".f95", ".f03", ".f08", ".f", ".fypp"] }

    fn grammar(&self, _lang_id: &str) -> Option<tree_sitter::Language> {
        Some(tree_sitter_fortran::LANGUAGE.into())
    }

    fn scope_kinds(&self) -> &[ScopeKind] { &[] }

    fn extract(&self, source: &str, file_path: &str, _lang_id: &str) -> ExtractionResult {
        // For .fypp files, attempt fypp preprocessing to generate concrete
        // per-type instantiations before tree-sitter sees the source.
        // If fypp is unavailable or fails, extract::extract falls back to
        // line-blanking the template directives — the same path used before.
        if file_path.ends_with(".fypp") {
            if let Some(expanded) = fypp::preprocess(file_path, source.as_bytes()) {
                return extract::extract(&expanded);
            }
        }
        extract::extract(source)
    }

    fn symbol_node_kinds(&self) -> &[&str] {
        &[
            "subroutine",
            "function",
            "module",
            "derived_type_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "use_statement",
            "subroutine_call",
            "call_expression",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] { keywords::KEYWORDS }
    // Note: Fortran is case-insensitive — KEYWORDS holds lowercase entries
    // and the resolver below also runs a manual case-folded check before
    // delegating, so refs like `INTEGER` / `integer` both classify.

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::FortranResolver))
    }

    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::FortranChecker))
    }
}
