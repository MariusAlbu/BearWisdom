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

use crate::ecosystem::manifest::{self, ManifestData, ManifestKind, PackageManifest};
use crate::ecosystem::{
    self, EcosystemActivation, EcosystemId, EcosystemRegistry, Platform,
};
use crate::languages::robot::dynamic_keywords::RobotDynamicKeywordMap;
use crate::languages::robot::library_map::{RobotLibraryMap, RobotResourceBasenameMap};
use crate::languages::vue::global_registry::VueGlobalRegistry;
use crate::types::PackageInfo;

// ---------------------------------------------------------------------------
// Re-exports — keep existing `project_context::parse_*` paths working
// ---------------------------------------------------------------------------

pub use crate::ecosystem::cargo::parse_cargo_dependencies;
pub use crate::ecosystem::composer::parse_composer_json_deps;
pub use crate::ecosystem::rubygems::parse_gemfile_gems;
pub use crate::ecosystem::go_mod::{find_go_mod, parse_go_mod, GoModData};
pub use crate::ecosystem::manifest::gradle::parse_gradle_dependencies;
pub use crate::ecosystem::manifest::maven::{extract_xml_text, parse_pom_xml_dependencies};
pub use crate::ecosystem::manifest::npm::parse_package_json_deps;
pub use crate::ecosystem::nuget::{
    implicit_usings_for_sdk, most_capable_sdk, parse_global_usings, parse_package_references,
    parse_project_references, parse_sdk_type, DotnetSdkType,
};
pub use crate::ecosystem::pypi::{
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

    /// Gradle version catalog accessor names discovered at `gradle/*.versions.toml`.
    ///
    /// For a file named `gradle/libs.versions.toml` the accessor name is `libs`.
    /// Kotlin build scripts reference these catalogs as `libs.androidx.core`,
    /// `libs.plugins.android.application`, `libs.versions.compileSdk`, etc.
    /// The Kotlin resolver uses this list to classify any ref whose root segment
    /// matches a catalog name as an external (build-tooling) reference.
    ///
    /// The conventional default is `["libs"]`; custom names are supported via
    /// additional `.versions.toml` files in `gradle/`.
    pub gradle_catalog_names: Vec<String>,

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

    /// Ecosystems whose `activation()` returned true for this project.
    ///
    /// Populated by `ProjectContext::initialize` (Phase 4). The full
    /// indexer iterates this list to drive externals discovery; resolvers
    /// filter by this set via `resolvable_ecosystems(lang)` at query time.
    ///
    /// Empty for contexts built via the legacy `build_project_context*`
    /// helpers — they predate the ecosystem seam and preserve old behavior
    /// (every ecosystem implicitly on). Callers that require the ecosystem
    /// seam must construct via `ProjectContext::initialize`.
    pub active_ecosystems: Vec<EcosystemId>,

    /// Language ids observed in the project's own files. Drives the
    /// `EcosystemActivation::LanguagePresent(lang)` predicate during
    /// initialization, and is itself a useful signal for diagnostics.
    pub language_presence: HashSet<String>,

    /// Vue-specific: project-wide globally-registered component map, populated
    /// after the file-scan pass in `full_index` (and the incremental equivalent).
    ///
    /// Used by `VueResolver::build_file_context` to inject synthetic import
    /// entries for components that are registered globally (via `app.use()`,
    /// `app.component()`, or `unplugin-vue-components`) so that the standard
    /// TS import-resolution chain can resolve them.
    ///
    /// `Default` leaves this empty; the full indexer populates it by calling
    /// `languages::vue::global_registry::scan_global_registrations`.
    pub vue_global_registry: VueGlobalRegistry,

    /// Robot Framework: per-file flattened Library imports keyed by
    /// `.robot`/`.resource` file path. Built by walking each file's
    /// Resource imports transitively and resolving every `Library  <name>`
    /// to a project-internal `.py` file. Used by `RobotResolver` to
    /// resolve cross-language keyword calls (e.g. `Check Test Case` →
    /// `check_test_case` in `TestCheckerLibrary.py`).
    ///
    /// `Default` leaves this empty; the full indexer populates it after
    /// parsing via `languages::robot::library_map::build_robot_library_map`.
    pub robot_library_map: RobotLibraryMap,

    /// Robot Framework: project-wide map from `.robot`/`.resource`
    /// basename to its indexed full path. The extractor can only see the
    /// bare filename in `Resource    atest_resource.robot`; the resolver
    /// uses this map to translate to the indexed path before calling
    /// `lookup.in_file(...)`. Without it, every cross-file Resource
    /// import silently misses Step 4.
    pub robot_resource_basenames: RobotResourceBasenameMap,

    /// Robot Framework: per-Python-library dynamic keywords. Each
    /// `.py` file referenced as a Robot Library is scanned for
    /// `KEYWORDS = {...}` dicts and `get_keyword_names` list-literal
    /// returns. Keys are project-relative `.py` paths; values are the
    /// normalised keyword names exposed by that file plus their owning
    /// class (or `None` for a module-level KEYWORDS dict). The
    /// `RobotResolver::build_file_context` step plumbs these into the
    /// per-file import list so resolution can reach keywords that have
    /// no `def name():` declaration.
    pub robot_dynamic_keywords: RobotDynamicKeywordMap,
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
    let gradle_catalog_names = discover_gradle_catalog_names(project_root);
    ProjectContext {
        manifests,
        by_package: HashMap::new(),
        workspace_pkg_by_declared_name: HashMap::new(),
        workspace_pkg_paths: HashMap::new(),
        gradle_catalog_names,
        active_ecosystems: Vec::new(),
        language_presence: HashSet::new(),
        vue_global_registry: VueGlobalRegistry::default(),
        robot_library_map: RobotLibraryMap::default(),
        robot_resource_basenames: RobotResourceBasenameMap::default(),
        robot_dynamic_keywords: RobotDynamicKeywordMap::default(),
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

    let gradle_catalog_names = discover_gradle_catalog_names(project_root);

    ProjectContext {
        manifests,
        by_package,
        workspace_pkg_by_declared_name,
        workspace_pkg_paths,
        gradle_catalog_names,
        active_ecosystems: Vec::new(),
        language_presence: HashSet::new(),
        vue_global_registry: VueGlobalRegistry::default(),
        robot_library_map: RobotLibraryMap::default(),
        robot_resource_basenames: RobotResourceBasenameMap::default(),
        robot_dynamic_keywords: RobotDynamicKeywordMap::default(),
    }
}

// ---------------------------------------------------------------------------
// Phase 4 seam — ProjectContext::initialize
// ---------------------------------------------------------------------------

/// Map from `EcosystemId` to the `ManifestKind`s that ecosystem owns. Used
/// to translate the legacy manifest-kind-indexed map into ecosystem-indexed
/// activation input. Ecosystems without any `ManifestKind` coverage
/// (currently cabal, nimble, cpan) fall back to language-presence-driven
/// activation — which is what their predicates request anyway.
#[allow(dead_code)] // wired in when ManifestMatch gains per-ecosystem scope
pub(crate) fn manifest_kinds_for_ecosystem(id: EcosystemId) -> &'static [ManifestKind] {
    match id.as_str() {
        "maven" => &[ManifestKind::Maven, ManifestKind::Gradle,
                     ManifestKind::Sbt, ManifestKind::Clojure],
        "npm" => &[ManifestKind::Npm],
        "pypi" => &[ManifestKind::PyProject],
        "cargo" => &[ManifestKind::Cargo],
        "hex" => &[ManifestKind::Mix, ManifestKind::Gleam],
        "nuget" => &[ManifestKind::NuGet],
        "spm" => &[ManifestKind::SwiftPM],
        "go-mod" => &[ManifestKind::GoMod],
        "rubygems" => &[ManifestKind::Gemfile],
        "composer" => &[ManifestKind::Composer],
        "cran" => &[ManifestKind::Description],
        "pub" => &[ManifestKind::Pubspec],
        "opam" => &[ManifestKind::Opam],
        "luarocks" => &[ManifestKind::Rockspec],
        "zig-pkg" => &[ManifestKind::ZigZon],
        _ => &[],
    }
}

impl ProjectContext {
    /// Phase 4 entry point. Constructs a fully populated `ProjectContext`:
    /// manifest scan + ecosystem activation evaluation + language presence
    /// detection. Prefer this over the legacy `build_project_context*`
    /// helpers in new code paths.
    ///
    /// Language presence is derived from the supplied `language_ids`
    /// (typically the distinct languages of parsed project files), so the
    /// caller doesn't re-walk the project. Passing an empty set disables
    /// `LanguagePresent(...)` activation and leaves only manifest-driven
    /// activations.
    pub fn initialize<I>(
        project_root: &Path,
        packages: &[PackageInfo],
        language_ids: I,
        ecosystems: &EcosystemRegistry,
    ) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let mut ctx = if packages.is_empty() {
            build_project_context(project_root)
        } else {
            build_project_context_with_packages(project_root, packages)
        };
        ctx.language_presence = language_ids.into_iter().collect();
        ctx.active_ecosystems = evaluate_active_ecosystems(&ctx, ecosystems);
        if !ctx.active_ecosystems.is_empty() {
            info!(
                "ProjectContext: {} active ecosystems ({})",
                ctx.active_ecosystems.len(),
                ctx.active_ecosystems
                    .iter()
                    .map(|e| e.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        ctx
    }

    /// Ecosystems that a ref produced from a `lang` file can reach. Filters
    /// `active_ecosystems` by each ecosystem's `languages()` capability.
    ///
    /// Returns an empty vec for contexts built via the legacy helpers (when
    /// `active_ecosystems` is empty) — callers that need honest filtering
    /// must use `ProjectContext::initialize`.
    pub fn resolvable_ecosystems(&self, lang: &str) -> Vec<EcosystemId> {
        if self.active_ecosystems.is_empty() {
            return Vec::new();
        }
        let reg = ecosystem::default_registry();
        self.active_ecosystems
            .iter()
            .copied()
            .filter(|id| {
                reg.get(*id)
                    .map(|e| e.languages().iter().any(|l| *l == lang))
                    .unwrap_or(false)
            })
            .collect()
    }
}

/// Walk every registered ecosystem, evaluate its `activation()` predicate
/// against the given project context, and return the active ids in
/// registration order.
fn evaluate_active_ecosystems(
    ctx: &ProjectContext,
    reg: &EcosystemRegistry,
) -> Vec<EcosystemId> {
    let mut active: Vec<EcosystemId> = Vec::new();
    // Two passes to handle `TransitiveOn(other)` without depending on
    // registration order: pass 1 resolves everything non-transitive, pass 2
    // resolves transitives against pass 1's output. Nested transitives
    // aren't supported; the trait doesn't use them today.
    for eco in reg.all() {
        if is_transitive_only(&eco.activation()) {
            continue;
        }
        if evaluate_activation(&eco.activation(), ctx, &active) {
            active.push(eco.id());
        }
    }
    for eco in reg.all() {
        if !is_transitive_only(&eco.activation()) {
            continue;
        }
        if active.contains(&eco.id()) {
            continue;
        }
        if evaluate_activation(&eco.activation(), ctx, &active) {
            active.push(eco.id());
        }
    }
    active
}

fn is_transitive_only(act: &EcosystemActivation) -> bool {
    matches!(act, EcosystemActivation::TransitiveOn(_))
}

fn evaluate_activation(
    act: &EcosystemActivation,
    ctx: &ProjectContext,
    already_active: &[EcosystemId],
) -> bool {
    match act {
        EcosystemActivation::Always => true,
        EcosystemActivation::Never => false,
        EcosystemActivation::ManifestMatch => ctx_has_manifest_for_current_ecosystem(ctx),
        EcosystemActivation::LanguagePresent(lang) => {
            ctx.language_presence.contains(*lang)
        }
        EcosystemActivation::ManifestFieldContains { .. } => {
            // Not evaluated in Phase 4 — no ecosystem uses this today.
            false
        }
        EcosystemActivation::AlwaysOnPlatform(plat) => matches_platform(*plat),
        EcosystemActivation::TransitiveOn(id) => already_active.contains(id),
        EcosystemActivation::All(clauses) => clauses
            .iter()
            .all(|c| evaluate_activation(c, ctx, already_active)),
        EcosystemActivation::Any(clauses) => clauses
            .iter()
            .any(|c| evaluate_activation(c, ctx, already_active)),
    }
}

/// Called with a `ManifestMatch` clause. We don't know which ecosystem we're
/// evaluating here (the activation enum is ecosystem-agnostic by design), so
/// we conservatively treat `ManifestMatch` as "some manifest was detected"
/// — unioned across every ecosystem kind. The ecosystem's own
/// `locate_roots` re-filters at discovery time, and in practice every
/// ecosystem's `activation` predicate is `Any([ManifestMatch, Language...])`
/// so the language clause carries the load when a manifest isn't mapped.
///
/// When Phase 5 adds stdlib ecosystems with pure `ManifestMatch` activations
/// against a specific manifest glob, this becomes the place to thread the
/// per-ecosystem manifest spec check through.
fn ctx_has_manifest_for_current_ecosystem(ctx: &ProjectContext) -> bool {
    !ctx.manifests.is_empty()
}

fn matches_platform(plat: Platform) -> bool {
    let is_windows = cfg!(target_os = "windows");
    let is_macos = cfg!(target_os = "macos");
    let is_linux = cfg!(target_os = "linux");
    match plat {
        Platform::Windows => is_windows,
        Platform::MacOs => is_macos,
        Platform::Linux => is_linux,
        Platform::Unix => !is_windows,
        Platform::AnyDesktop => is_windows || is_macos || is_linux,
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
// Gradle version catalog discovery
// ---------------------------------------------------------------------------

/// Scan `{project_root}/gradle/` for `*.versions.toml` files and return the
/// catalog accessor names derived from their stems.
///
/// Convention: `gradle/libs.versions.toml` → accessor name `libs`.
/// Gradle supports multiple catalogs; each `.versions.toml` file in the
/// `gradle/` directory registers one. We walk up to two levels of subdirectories
/// to handle nested Gradle projects (Android multi-module layouts).
pub(crate) fn discover_gradle_catalog_names(project_root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    discover_gradle_catalog_names_recursive(project_root, &mut names, 0);
    names.dedup();
    names
}

fn discover_gradle_catalog_names_recursive(dir: &Path, names: &mut Vec<String>, depth: usize) {
    if depth > 3 {
        return;
    }
    let gradle_dir = dir.join("gradle");
    if gradle_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&gradle_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                        // e.g. "libs.versions.toml" → stem before ".versions.toml" = "libs"
                        if let Some(stem) = file_name.strip_suffix(".versions.toml") {
                            if !stem.is_empty() {
                                names.push(stem.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    // Walk one level deeper for Android multi-module layouts where the catalog
    // may live in a subproject's gradle/ dir (e.g. noty-android/gradle/).
    if depth < 2 {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if matches!(name, ".git" | "build" | "target" | ".gradle" | "node_modules") {
                            continue;
                        }
                    }
                    discover_gradle_catalog_names_recursive(&path, names, depth + 1);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "project_context_tests.rs"]
mod tests;
