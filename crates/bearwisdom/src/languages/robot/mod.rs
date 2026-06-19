//! Robot Framework language plugin.
//!
//! Grammar: no tree-sitter grammar in Cargo.toml.
//! `grammar()` returns `None`; extraction uses a line-oriented parser that
//! recognises Robot Framework's section-based structure.

pub mod dynamic_keywords;
pub mod extract;
pub mod keywords;
pub mod library_map;
mod predicates;
pub(crate) mod profile;
pub use profile::ROBOT_PROFILE;

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;

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
    fn id(&self) -> &str {
        "robot"
    }

    fn language_ids(&self) -> &[&str] {
        &["robot"]
    }

    fn extensions(&self) -> &[&str] {
        &[".robot", ".resource"]
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
        &[
            "keyword_definition",
            "test_case_definition",
            "variable_definition",
        ]
    }

    fn ref_node_kinds(&self) -> &[&str] {
        &["keyword_invocation", "setting_statement"]
    }

    fn keywords(&self) -> &'static [&'static str] {
        &[]
    }

    fn profile(
        &self,
    ) -> Option<&'static crate::type_checker::profile::language_profile::LanguageProfile> {
        Some(&profile::ROBOT_PROFILE)
    }

    fn populate_project_state(
        &self,
        state: &mut PluginStateBag,
        parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &ProjectContext,
    ) {
        let root = project_root.to_path_buf();
        state.set(build_robot_project_state(parsed, |path| {
            std::fs::read_to_string(root.join(path)).ok()
        }));
    }

    fn populate_project_state_post_externals(
        &self,
        state: &mut PluginStateBag,
        parsed: &[ParsedFile],
        project_root: &std::path::Path,
        _project_ctx: &ProjectContext,
    ) {
        // `parsed` now carries the externally-walked Python files (e.g. a
        // pip-installed `SeleniumLibrary/__init__.py` + its `keywords/*.py`
        // member modules). Rebuilding here lets `Library  SeleniumLibrary`
        // bind to the site-packages package and its DynamicCore keyword
        // methods, which the pre-externals pass couldn't see.
        //
        // `ext:` library files have no on-disk twin under `project_root`; the
        // robot-externals pull records their absolute paths in the bag's
        // `RobotExternalSources` (set by the indexer before this hook runs)
        // so the dynamic-keyword scan can read them.
        let root = project_root.to_path_buf();
        let abs_by_virtual = state
            .get::<RobotExternalSources>()
            .map(|s| s.abs_by_virtual.clone())
            .unwrap_or_default();
        state.set(build_robot_project_state(parsed, move |path| {
            if let Some(abs) = abs_by_virtual.get(path) {
                return std::fs::read_to_string(abs).ok();
            }
            if path.starts_with("ext:") {
                return None;
            }
            std::fs::read_to_string(root.join(path)).ok()
        }));
    }
}

/// Absolute-path index for robot-demanded external library files.
///
/// The robot externals pull walks site-packages packages a suite declares
/// via `Library  <name>`; each walked file carries a virtual `ext:py:...`
/// path (used as the symbol-index key) and an absolute on-disk path. The
/// post-externals state rebuild reads keyword-method source from disk, so it
/// needs the virtual→absolute mapping the walk produced.
#[derive(Default)]
pub struct RobotExternalSources {
    pub abs_by_virtual: std::collections::HashMap<String, std::path::PathBuf>,
}

/// Build the cross-file Robot state from a parsed-file slice.
///
/// Reads library/resource binding maps, then scans each resolved library
/// `.py` file — plus, for a package entry point, its member modules — for
/// dynamic-library keyword definitions. `read_source` abstracts over the file
/// source: project files read from disk, `ext:` files read through the
/// absolute paths recorded by the externals pull.
fn build_robot_project_state(
    parsed: &[ParsedFile],
    read_source: impl Fn(&str) -> Option<String>,
) -> RobotProjectState {
    let lib_map = library_map::build_robot_library_map(parsed);

    // For each resolved library path, gather its package member modules (empty
    // for flat modules). `entries` drives a member-aware dynamic-keyword scan
    // that attributes every member's keywords to the library entry path.
    let mut library_paths: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for libs in lib_map.values() {
        for lib in libs {
            library_paths.insert(lib.py_file_path.as_str());
        }
    }
    let entries: Vec<(&str, Vec<&str>)> = library_paths
        .iter()
        .copied()
        .map(|path| (path, library_map::package_member_modules(path, parsed)))
        .collect();

    let dyn_kw_map =
        dynamic_keywords::build_robot_dynamic_keyword_map_with_members(&entries, read_source);

    RobotProjectState {
        library_map: lib_map,
        resource_basenames: library_map::build_robot_resource_basename_map(parsed),
        dynamic_keywords: dyn_kw_map,
    }
}
