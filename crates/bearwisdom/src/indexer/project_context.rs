// =============================================================================
// indexer/project_context.rs — Data-driven project context for resolution
//
// Scans manifest files (package.json, Cargo.toml, .csproj, go.mod, etc.) to
// build per-project knowledge about external dependencies and SDK configuration.
// This replaces hardcoded type/namespace maps with project-level data.
//
// The actual parsing logic lives in `manifest/` submodules. This module owns
// the `ProjectContext` type and the `build_project_context` entry point.
// Public parse functions are re-exported here for backward compatibility with
// existing resolver and test code that imports them via this path.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::Path;
use tracing::{debug, info};

use super::manifest::{self, ManifestData, ManifestKind};

// ---------------------------------------------------------------------------
// Re-exports — keep existing `project_context::parse_*` paths working
// ---------------------------------------------------------------------------

pub use super::manifest::cargo::parse_cargo_dependencies;
pub use super::manifest::composer::parse_composer_json_deps;
pub use super::manifest::gemfile::parse_gemfile_gems;
pub use super::manifest::go_mod::{find_go_mod, parse_go_mod, GoModData};
pub use super::manifest::gradle::parse_gradle_dependencies;
pub use super::manifest::maven::{extract_xml_text, parse_pom_xml_dependencies};
pub use super::manifest::npm::parse_package_json_deps;
pub use super::manifest::nuget::{
    implicit_usings_for_sdk, most_capable_sdk, parse_global_usings, parse_package_references,
    parse_sdk_type, DotnetSdkType,
};
pub use super::manifest::pyproject::{
    parse_pipfile_deps, parse_pyproject_deps, parse_requirements_txt,
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Project-level context built once per index, used by language resolvers
/// to classify external references based on manifest data.
///
/// Language-specific classification logic lives in each language plugin's
/// resolver, not here. This struct is a dumb data holder.
#[derive(Debug, Clone, Default)]
pub struct ProjectContext {
    /// Raw manifest data keyed by ecosystem, populated by `read_all_manifests`.
    /// Each language resolver reads the manifest(s) it cares about:
    ///   - TypeScript/JS → `ManifestKind::Npm`
    ///   - Rust          → `ManifestKind::Cargo`
    ///   - C#/F#/VB.NET  → `ManifestKind::NuGet`
    ///   - Go            → `ManifestKind::GoMod`
    ///   - Python        → `ManifestKind::PyProject`
    ///   - Ruby          → `ManifestKind::Gemfile`
    ///   - PHP           → `ManifestKind::Composer`
    ///   - Java/Kotlin   → `ManifestKind::Maven` / `ManifestKind::Gradle`
    ///   - Elixir        → `ManifestKind::Mix`
    ///   - Swift         → `ManifestKind::SwiftPM`
    ///   - Dart/Flutter  → `ManifestKind::Pubspec`
    pub manifests: HashMap<ManifestKind, ManifestData>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a `ProjectContext` by scanning the project root for manifest files.
///
/// Uses the registered `ManifestReader` implementations in `manifest/` to
/// locate and parse all ecosystem manifests, then back-fills the per-language
pub fn build_project_context(project_root: &Path) -> ProjectContext {
    let manifests = manifest::read_all_manifests(project_root);

    if let Some(go) = manifests.get(&ManifestKind::GoMod) {
        if let Some(ref module_path) = go.module_path {
            debug!("Go module path: {module_path}");
        }
    }

    let total_deps: usize = manifests.values().map(|m| m.dependencies.len()).sum();
    info!(
        "ProjectContext: {} manifests, {} total dependencies",
        manifests.len(),
        total_deps,
    );

    ProjectContext { manifests }
}

impl ProjectContext {
    /// Get raw manifest data for a specific ecosystem.
    pub fn manifest(&self, kind: ManifestKind) -> Option<&ManifestData> {
        self.manifests.get(&kind)
    }

    /// Check whether a specific ecosystem manifest declared a dependency.
    pub fn has_dependency(&self, kind: ManifestKind, name: &str) -> bool {
        self.manifests
            .get(&kind)
            .map_or(false, |m| m.dependencies.contains(name))
    }

    /// Collect all dependency names from every ecosystem manifest into a single set.
    pub fn all_dependency_names(&self) -> HashSet<String> {
        let mut deps = HashSet::new();
        for m in self.manifests.values() {
            deps.extend(m.dependencies.iter().cloned());
        }
        deps
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "project_context_tests.rs"]
mod tests;
