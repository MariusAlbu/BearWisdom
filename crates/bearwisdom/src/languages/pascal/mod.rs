//! Pascal / Delphi language plugin.
//!
//! Grammar: tree-sitter-pascal 0.10.2 — real grammar, LANGUAGE constant available.

mod decls;
mod error_recovery;
pub mod extract;
mod include_directives;
pub mod keywords;
pub(crate) mod main_unit;
mod normalise;
mod qualify;
mod refs;
mod predicates;
pub(crate) mod profile;
pub use profile::PASCAL_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::resolve::engine::contract::ImportEntry;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{ExtractionResult, ParsedFile};

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
        &["exprCall", "declUses", "typeref"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        keywords::KEYWORDS
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::PASCAL_PROFILE)
    }

    fn populate_project_state(
        &self,
        state: &mut PluginStateBag,
        parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &crate::indexer::project_context::ProjectContext,
    ) {
        state.set(main_unit::build_main_unit_state(parsed, project_root));
    }

    /// `{%MainUnit}` fragment scope inheritance: a `.inc` fragment spliced
    /// into a parent unit via `{$I}` carries no `uses` clause of its own, so
    /// this redirects the parent unit's own `uses` list — plus the parent
    /// unit's own name, for siblings spliced into the same unit — into the
    /// fragment's wildcard scope. See `main_unit` for the cross-file state
    /// this reads.
    fn extra_wildcard_imports(&self, state: &PluginStateBag, file: &ParsedFile) -> Vec<ImportEntry> {
        let Some(project_state) = state.get::<main_unit::PascalProjectState>() else {
            return Vec::new();
        };
        project_state
            .wildcards_for(&file.path)
            .iter()
            .map(|name| ImportEntry {
                imported_name: name.clone(),
                module_path: Some(name.clone()),
                alias: None,
                is_wildcard: true,
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod mod_tests;
