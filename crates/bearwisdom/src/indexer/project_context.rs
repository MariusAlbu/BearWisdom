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
use std::path::{Path, PathBuf};
use tracing::{debug, info};

use super::manifest::{self, ManifestData, ManifestKind, PackageManifest};
use crate::types::PackageInfo;

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
    parse_project_references, parse_sdk_type, DotnetSdkType,
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
    /// Raw manifest data keyed by ecosystem — the **union** across the whole
    /// project. Used as the fallback for files whose `package_id` is None
    /// (root configs, shared scripts, misc) and for single-project layouts
    /// where `by_package` is empty.
    ///
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

    /// Per-package manifests — populated only for monorepos (M2).
    ///
    /// Key: `packages.id` from the DB. Value: that package's own manifests,
    /// NOT unioned with siblings. An empty map means either (a) single-package
    /// project, or (b) the resolver was built via the legacy builder that
    /// doesn't know about package rows.
    ///
    /// Callers that need per-package classification must use
    /// `manifests_for(package_id)` which transparently falls back to the
    /// union when the package isn't in the map.
    pub by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>>,

    /// Workspace packages keyed by `declared_name` (the manifest-reported
    /// name, e.g. `@myorg/utils` from `package.json`).
    ///
    /// Populated only by `build_project_context_with_packages`. A hit means
    /// the module specifier refers to a sibling workspace package — the
    /// TypeScript (and other) resolvers use this to produce confidence-1.0
    /// cross-package edges instead of classifying as external.
    ///
    /// Packages with no `declared_name` (e.g. .csproj without AssemblyName
    /// metadata, or manifests that couldn't be parsed) are absent from
    /// this map.
    pub workspace_pkg_by_declared_name: HashMap<String, i64>,

    /// Map from `package_id` → package's relative path (e.g. `apps/landing`).
    /// Populated alongside `workspace_pkg_by_declared_name`. Used to resolve
    /// package-relative paths like tsconfig `paths` targets, which are
    /// specified relative to each package's own directory, not the
    /// workspace root.
    pub workspace_pkg_paths: HashMap<i64, String>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a `ProjectContext` by scanning the project root for manifest files.
///
/// This is the legacy single-unit builder. It populates only `manifests`
/// (the union). `by_package` stays empty, so per-package queries fall back
/// to the union — same observable behavior as pre-M2.
///
/// Single-project callers (incremental indexer, tests) use this. The full
/// indexer uses `build_project_context_with_packages` when workspace
/// packages were detected.
pub fn build_project_context(project_root: &Path) -> ProjectContext {
    let manifests = manifest::read_all_manifests(project_root);
    log_manifests(&manifests);
    ProjectContext {
        manifests,
        by_package: HashMap::new(),
        workspace_pkg_by_declared_name: HashMap::new(),
        workspace_pkg_paths: HashMap::new(),
    }
}

/// Build a `ProjectContext` that includes per-package manifests keyed by
/// `packages.id`.
///
/// For each `PackageInfo` with a DB `id`, this walks the per-package manifest
/// output from `read_all_manifests_per_package` and collects only the manifests
/// whose relative path matches the package's relative path. The result is
/// one ecosystem map per package.
///
/// Files with `package_id == None` (root configs, shared scripts) use the
/// legacy `manifests` union as their context.
pub fn build_project_context_with_packages(
    project_root: &Path,
    packages: &[PackageInfo],
) -> ProjectContext {
    let per_package_manifests = manifest::read_all_manifests_per_package(project_root);

    // Union map — same as the legacy builder.
    let manifests = union_manifests(&per_package_manifests);

    // Per-package map — index by DB id, matching against PackageInfo.path.
    let mut by_package: HashMap<i64, HashMap<ManifestKind, ManifestData>> = HashMap::new();
    for pkg in packages {
        let Some(id) = pkg.id else { continue };
        let pkg_path = PathBuf::from(&pkg.path);
        let mut pkg_manifests: HashMap<ManifestKind, ManifestData> = HashMap::new();
        for pm in &per_package_manifests {
            if pm.path == pkg_path {
                // Only the manifests whose package_dir exactly matches this
                // package — prevents a parent directory's manifest from
                // leaking down into a child package's context.
                let entry = pkg_manifests.entry(pm.kind).or_default();
                entry.dependencies.extend(pm.data.dependencies.iter().cloned());
                if pm.data.module_path.is_some() {
                    entry.module_path = pm.data.module_path.clone();
                }
                entry
                    .global_usings
                    .extend(pm.data.global_usings.iter().cloned());
                if pm.data.sdk_type.is_some() {
                    entry.sdk_type = pm.data.sdk_type.clone();
                }
                for pr in &pm.data.project_refs {
                    if !entry.project_refs.contains(pr) {
                        entry.project_refs.push(pr.clone());
                    }
                }
                for alias in &pm.data.tsconfig_paths {
                    if !entry.tsconfig_paths.contains(alias) {
                        entry.tsconfig_paths.push(alias.clone());
                    }
                }
            }
        }
        if !pkg_manifests.is_empty() {
            by_package.insert(id, pkg_manifests);
        }
    }

    // Workspace declared_name → package_id index. Only packages that actually
    // reported a name in their manifest participate — folder-derived `name`
    // is never used here because it doesn't match what imports will reference.
    let mut workspace_pkg_by_declared_name: HashMap<String, i64> = HashMap::new();
    let mut workspace_pkg_paths: HashMap<i64, String> = HashMap::new();
    for pkg in packages {
        let Some(id) = pkg.id else { continue };
        workspace_pkg_paths.insert(id, pkg.path.clone());
        let Some(declared) = &pkg.declared_name else { continue };
        if declared.is_empty() {
            continue;
        }
        workspace_pkg_by_declared_name.insert(declared.clone(), id);
    }

    log_manifests(&manifests);
    info!(
        "ProjectContext: per-package contexts for {}/{} workspace packages, {} declared names indexed",
        by_package.len(),
        packages.len(),
        workspace_pkg_by_declared_name.len(),
    );

    ProjectContext {
        manifests,
        by_package,
        workspace_pkg_by_declared_name,
        workspace_pkg_paths,
    }
}

fn union_manifests(per_package: &[PackageManifest]) -> HashMap<ManifestKind, ManifestData> {
    let mut out: HashMap<ManifestKind, ManifestData> = HashMap::new();
    for pm in per_package {
        let entry = out.entry(pm.kind).or_default();
        entry.dependencies.extend(pm.data.dependencies.iter().cloned());
        if pm.data.module_path.is_some() {
            entry.module_path = pm.data.module_path.clone();
        }
        entry
            .global_usings
            .extend(pm.data.global_usings.iter().cloned());
        if pm.data.sdk_type.is_some() {
            entry.sdk_type = pm.data.sdk_type.clone();
        }
        for pr in &pm.data.project_refs {
            if !entry.project_refs.contains(pr) {
                entry.project_refs.push(pr.clone());
            }
        }
        for alias in &pm.data.tsconfig_paths {
            if !entry.tsconfig_paths.contains(alias) {
                entry.tsconfig_paths.push(alias.clone());
            }
        }
    }
    out
}

fn log_manifests(manifests: &HashMap<ManifestKind, ManifestData>) {
    if let Some(go) = manifests.get(&ManifestKind::GoMod) {
        if let Some(ref module_path) = go.module_path {
            debug!("Go module path: {module_path}");
        }
    }
    let total_deps: usize = manifests.values().map(|m| m.dependencies.len()).sum();
    info!(
        "ProjectContext: {} manifests, {} total dependencies (union)",
        manifests.len(),
        total_deps,
    );
}

impl ProjectContext {
    /// Get raw manifest data for a specific ecosystem (the union across the
    /// whole project).
    pub fn manifest(&self, kind: ManifestKind) -> Option<&ManifestData> {
        self.manifests.get(&kind)
    }

    /// Check whether a specific ecosystem manifest declared a dependency
    /// (union — not per-package).
    pub fn has_dependency(&self, kind: ManifestKind, name: &str) -> bool {
        self.manifests
            .get(&kind)
            .map_or(false, |m| m.dependencies.contains(name))
    }

    /// Collect all dependency names from every ecosystem manifest into a
    /// single set (union).
    pub fn all_dependency_names(&self) -> HashSet<String> {
        let mut deps = HashSet::new();
        for m in self.manifests.values() {
            deps.extend(m.dependencies.iter().cloned());
        }
        deps
    }

    /// Return the manifest set visible to a file with the given `package_id`.
    ///
    /// For files with a known package_id that has per-package data, returns
    /// only that package's manifests — no cross-package pollution. For
    /// files with no package_id (root configs) or when per-package data
    /// is unavailable, returns the union.
    ///
    /// This is the primary entry point for per-package classification
    /// in the resolver's external-ref pipeline.
    pub fn manifests_for(
        &self,
        package_id: Option<i64>,
    ) -> &HashMap<ManifestKind, ManifestData> {
        if let Some(id) = package_id {
            if let Some(pkg_manifests) = self.by_package.get(&id) {
                return pkg_manifests;
            }
        }
        &self.manifests
    }

    /// Check whether a specific dep name is declared in the manifest visible
    /// to `package_id` — respects per-package isolation when available.
    pub fn has_dependency_for(
        &self,
        package_id: Option<i64>,
        kind: ManifestKind,
        name: &str,
    ) -> bool {
        self.manifests_for(package_id)
            .get(&kind)
            .map_or(false, |m| m.dependencies.contains(name))
    }

    /// Whether this context carries per-package data. False for single-project
    /// layouts and for contexts built via the legacy `build_project_context`.
    pub fn is_per_package(&self) -> bool {
        !self.by_package.is_empty()
    }

    /// Rewrite a TS import specifier through the given package's tsconfig
    /// `paths` aliases.
    ///
    /// Returns the longest-matching alias prefix's rewrite, or `None` if no
    /// alias matches. The target is a bare prefix (trailing `*` already
    /// stripped), so `@/utils` with alias `@/ -> src/` becomes `src/utils`.
    /// Consumers typically drop the result into `SymbolLookup::in_file`
    /// (via the module_to_file map) or use it as a filename-stem match.
    pub fn resolve_tsconfig_alias(
        &self,
        package_id: Option<i64>,
        specifier: &str,
    ) -> Option<String> {
        let manifests = self.manifests_for(package_id);
        let npm = manifests.get(&ManifestKind::Npm)?;
        // Prefer the longest matching alias to handle nested aliases like
        //   "@/":           "src/"
        //   "@/components/": "packages/ui/src/"
        let mut best: Option<&(String, String)> = None;
        for entry in &npm.tsconfig_paths {
            let (alias, _) = entry;
            if specifier.starts_with(alias.as_str()) {
                if best.map_or(true, |(b, _)| alias.len() > b.len()) {
                    best = Some(entry);
                }
            }
        }
        let (alias, target) = best?;
        let remainder = &specifier[alias.len()..];
        Some(format!("{target}{remainder}"))
    }

    /// Resolve a module specifier (e.g. `@myorg/utils`, `@myorg/utils/sub/mod`)
    /// to a workspace `package_id` via `declared_name`.
    ///
    /// Matches exact first, then strips trailing path segments to handle deep
    /// imports: `@myorg/utils/sub/mod` → `@myorg/utils` → `@myorg`.
    ///
    /// Returns `None` if no workspace package declared that name, including
    /// single-project contexts where `workspace_pkg_by_declared_name` is empty.
    pub fn workspace_package_id(&self, specifier: &str) -> Option<i64> {
        if let Some(&id) = self.workspace_pkg_by_declared_name.get(specifier) {
            return Some(id);
        }
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if let Some(&id) = self.workspace_pkg_by_declared_name.get(path) {
                return Some(id);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "project_context_tests.rs"]
mod tests;
