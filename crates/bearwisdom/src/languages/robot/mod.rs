//! Robot Framework language plugin.
//!
//! Grammar: no tree-sitter grammar in Cargo.toml.
//! `grammar()` returns `None`; extraction uses a line-oriented parser that
//! recognises Robot Framework's section-based structure.

pub mod keywords;
pub mod extract;
pub mod library_map;
pub mod dynamic_keywords;
pub mod resolve;
mod predicates;
pub(crate) mod type_checker;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod resolve_tests;

#[cfg(test)]
#[path = "library_map_tests.rs"]
mod library_map_tests;

use crate::indexer::plugin_state::PluginStateBag;
use crate::indexer::project_context::ProjectContext;
use crate::languages::LanguagePlugin;
use crate::parser::scope_tree::ScopeKind;
use crate::types::{ExtractionResult, ParsedFile};

/// Cross-file Robot Framework state collected once per index pass.
///
/// Resolvers access this via `project_ctx.plugin_state.get::<RobotProjectState>()`.
pub struct RobotProjectState {
    pub library_map: library_map::RobotLibraryMap,
    pub resource_basenames: library_map::RobotResourceBasenameMap,
    pub dynamic_keywords: dynamic_keywords::RobotDynamicKeywordMap,
}

pub struct RobotPlugin;

impl LanguagePlugin for RobotPlugin {
    fn id(&self) -> &str { "robot" }

    fn language_ids(&self) -> &[&str] { &["robot"] }

    fn extensions(&self) -> &[&str] { &[".robot", ".resource"] }

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
        &[
            "keyword_definition",
            "test_case_definition",
            "variable_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &[
            "keyword_invocation",
            "setting_statement",
        ]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn resolver(&self) -> Option<std::sync::Arc<dyn crate::indexer::resolve::engine::LanguageResolver>> {
        Some(std::sync::Arc::new(resolve::RobotResolver))
    }


    fn type_checker(&self) -> Option<std::sync::Arc<dyn crate::type_checker::TypeChecker>> {
        Some(std::sync::Arc::new(type_checker::RobotChecker))
    }

    fn populate_project_state(
        &self,
        state: &mut PluginStateBag,
        parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &ProjectContext,
    ) {
        let lib_map = library_map::build_robot_library_map(parsed);
        let mut library_paths: std::collections::HashSet<&str> =
            std::collections::HashSet::new();
        for libs in lib_map.values() {
            for lib in libs {
                library_paths.insert(lib.py_file_path.as_str());
            }
        }
        let library_paths_vec: Vec<&str> = library_paths.iter().copied().collect();
        let dyn_kw_map = dynamic_keywords::build_robot_dynamic_keyword_map(
            &library_paths_vec,
            |path| std::fs::read_to_string(project_root.join(path)).ok(),
        );
        state.set(RobotProjectState {
            library_map: lib_map,
            resource_basenames: library_map::build_robot_resource_basename_map(parsed),
            dynamic_keywords: dyn_kw_map,
        });
    }
}
