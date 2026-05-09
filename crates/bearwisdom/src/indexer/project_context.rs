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
    /// Project root directory. Populated by the production builders
    /// (`build_project_context*`); defaults to an empty `PathBuf` for
    /// `Default`-constructed instances. Used by the activation evaluator
    /// to scan the project for files when a `ManifestFieldContains` clause
    /// is encountered, and is available for any other ecosystem code that
    /// needs the absolute project root.
    pub project_root: PathBuf,

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
    /// Populated by `ProjectContext::initialize`. For polyglot monorepos,
    /// this is the union of `active_ecosystems_by_package` so legacy
    /// workspace-wide consumers keep working unchanged.
    ///
    /// Empty for contexts built via the legacy `build_project_context*`
    /// helpers — they predate the ecosystem seam and preserve old behavior
    /// (every ecosystem implicitly on). Callers that require the ecosystem
    /// seam must construct via `ProjectContext::initialize`.
    pub active_ecosystems: Vec<EcosystemId>,

    /// Per-package active ecosystems, evaluated against each package's
    /// own manifest set and absolute path. Empty for single-project layouts
    /// and for legacy contexts.
    ///
    /// Closes the workspace-flat activation gap: a frontend `tsconfig.json`
    /// declaring `"DOM"` no longer activates `ts_lib_dom` for an unrelated
    /// backend package in the same repo. Consumers with a `package_id` in
    /// hand should consult this map; everything else falls back to
    /// `active_ecosystems`.
    pub active_ecosystems_by_package: HashMap<i64, Vec<EcosystemId>>,

    /// Language ids observed in the project's own files. Drives the
    /// `EcosystemActivation::LanguagePresent(lang)` predicate during
    /// initialization, and is itself a useful signal for diagnostics.
    pub language_presence: HashSet<String>,

    /// Per-package language presence — the languages observed in files
    /// owned by each workspace package. Populated by
    /// `ProjectContext::initialize` when the caller buckets the parsed
    /// file slice by package path. Empty for single-project layouts and
    /// for callers that don't have parsed files in hand (incremental
    /// indexer).
    ///
    /// When non-empty, the per-package activation evaluator (Phase 5)
    /// consults this map for `LanguagePresent` clauses instead of
    /// `language_presence`. A polyglot monorepo with one Kotlin package
    /// and one Python package no longer activates `kotlin-stdlib` for
    /// the Python package's locator pass.
    pub language_presence_by_package: HashMap<i64, HashSet<String>>,

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
        project_root: project_root.to_path_buf(),
        manifests,
        by_package: HashMap::new(),
        workspace_pkg_by_declared_name: HashMap::new(),
        workspace_pkg_paths: HashMap::new(),
        gradle_catalog_names,
        active_ecosystems: Vec::new(),
        active_ecosystems_by_package: HashMap::new(),
        language_presence: HashSet::new(),
        language_presence_by_package: HashMap::new(),
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
        project_root: project_root.to_path_buf(),
        manifests,
        by_package,
        workspace_pkg_by_declared_name,
        workspace_pkg_paths,
        gradle_catalog_names,
        active_ecosystems: Vec::new(),
        active_ecosystems_by_package: HashMap::new(),
        language_presence: HashSet::new(),
        language_presence_by_package: HashMap::new(),
        vue_global_registry: VueGlobalRegistry::default(),
        robot_library_map: RobotLibraryMap::default(),
        robot_resource_basenames: RobotResourceBasenameMap::default(),
        robot_dynamic_keywords: RobotDynamicKeywordMap::default(),
    }
}

// ---------------------------------------------------------------------------
// Phase 4 seam — ProjectContext::initialize
// ---------------------------------------------------------------------------

/// Map from `EcosystemId` to the `ManifestKind`s that ecosystem owns.
///
/// Drives the per-ecosystem `ManifestMatch` activation: an ecosystem's
/// `ManifestMatch` clause is satisfied iff at least one of its kinds is
/// present in `ProjectContext::manifests`.
///
/// Ecosystems without any `ManifestKind` coverage (cabal, nimble, cpan and
/// stdlib-shape entries) return an empty slice; their `ManifestMatch`
/// clause then evaluates to `false` and they must rely on a sibling
/// `LanguagePresent` / `TransitiveOn` clause inside an `Any(...)`
/// composite to activate.
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
        "puppet-forge" => &[ManifestKind::Puppet],
        "cabal" => &[ManifestKind::Cabal],
        "cpan" => &[ManifestKind::Cpan],
        "nimble" => &[ManifestKind::Nimble],
        "alire" => &[ManifestKind::Alire],
        "psgallery" => &[ManifestKind::Psd1],
        "bazel-central-registry" => &[ManifestKind::ModuleBazel],
        "tf-registry" => &[ManifestKind::Terraform],
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
        Self::initialize_with_per_package_languages(
            project_root,
            packages,
            language_ids,
            HashMap::new(),
            ecosystems,
        )
    }

    /// Phase 5 entry point. Same as `initialize` but takes an explicit
    /// per-package language map so `LanguagePresent` clauses narrow per
    /// package. Callers without parsed-file context (incremental
    /// indexer, tests) use the simpler `initialize` overload, which
    /// passes an empty map and falls back to workspace-wide language
    /// presence.
    pub fn initialize_with_per_package_languages<I>(
        project_root: &Path,
        packages: &[PackageInfo],
        language_ids: I,
        language_presence_by_package: HashMap<i64, HashSet<String>>,
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
        ctx.language_presence_by_package = language_presence_by_package;
        if packages.is_empty() {
            ctx.active_ecosystems = evaluate_active_ecosystems(&ctx, ecosystems);
        } else {
            ctx.active_ecosystems_by_package =
                evaluate_active_ecosystems_per_package(&ctx, ecosystems, packages);
            ctx.active_ecosystems = union_per_package_actives(
                &ctx.active_ecosystems_by_package,
                ecosystems,
            );
        }
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
        if evaluate_activation(&eco.activation(), eco.id(), ctx, &active) {
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
        if evaluate_activation(&eco.activation(), eco.id(), ctx, &active) {
            active.push(eco.id());
        }
    }
    active
}

fn is_transitive_only(act: &EcosystemActivation) -> bool {
    matches!(act, EcosystemActivation::TransitiveOn(_))
}

/// Evaluate ecosystem activation once per workspace package.
///
/// Each package gets its own scope: package-local manifests via
/// `ctx.manifests_for(Some(pkg_id))`, the package's absolute path as the
/// `ManifestFieldContains` glob root, and (for now) workspace-wide
/// language presence. The two-pass transitive resolution mirrors the
/// workspace-wide evaluator so `TransitiveOn(other)` clauses see the
/// non-transitive actives that fired for the same package.
///
/// Packages without a stable `pkg.id` are skipped silently — they have
/// no row in `by_package` and fall back to the union via
/// `active_ecosystems` automatically.
pub(crate) fn evaluate_active_ecosystems_per_package(
    ctx: &ProjectContext,
    reg: &EcosystemRegistry,
    packages: &[PackageInfo],
) -> HashMap<i64, Vec<EcosystemId>> {
    let mut out: HashMap<i64, Vec<EcosystemId>> = HashMap::new();
    for pkg in packages {
        let Some(pkg_id) = pkg.id else { continue };
        let pkg_abs = ctx.project_root.join(&pkg.path);
        // Per-package language presence when populated (Phase 5 caller
        // path), else workspace-wide. The workspace-wide fallback is
        // conservative and correct: `LanguagePresent` clauses fire on
        // any language present anywhere, but `eco.languages()` filters
        // out at query time.
        let pkg_langs = ctx
            .language_presence_by_package
            .get(&pkg_id)
            .unwrap_or(&ctx.language_presence);
        let scope = ActivationScope {
            manifests: ctx.manifests_for(Some(pkg_id)),
            language_presence: pkg_langs,
            glob_root: &pkg_abs,
        };
        let mut active: Vec<EcosystemId> = Vec::new();
        for eco in reg.all() {
            // Workspace-global ecosystems bypass the per-package scope —
            // they are evaluated below against the workspace-wide view.
            if eco.is_workspace_global() { continue; }
            if is_transitive_only(&eco.activation()) {
                continue;
            }
            if evaluate_activation_scoped(&eco.activation(), eco.id(), &scope, &active) {
                active.push(eco.id());
            }
        }
        for eco in reg.all() {
            if eco.is_workspace_global() { continue; }
            if !is_transitive_only(&eco.activation()) {
                continue;
            }
            if active.contains(&eco.id()) {
                continue;
            }
            if evaluate_activation_scoped(&eco.activation(), eco.id(), &scope, &active) {
                active.push(eco.id());
            }
        }
        out.insert(pkg_id, active);
    }

    // Workspace-global pass: ecosystems whose `is_workspace_global()` is
    // true describe workspace-level artefacts (compile_commands.json, OS
    // SDK headers, Qt install) that cover the entire build regardless of
    // how packages were detected. They activate against the workspace-wide
    // scope (union manifests + workspace-wide language presence + project
    // root) and are injected into every package's active list so the
    // discovery loop in `stage_link.rs` invokes their locator.
    //
    // Without this pass, projects whose actual codebase isn't covered by
    // a detected workspace package (e.g. a CMake C++ project where the
    // only detected package is a small Go utility under `utils/`) would
    // silently fail to discover the build's compile-DB, SDK headers, etc.
    let workspace_scope = ActivationScope {
        manifests: &ctx.manifests,
        language_presence: &ctx.language_presence,
        glob_root: &ctx.project_root,
    };
    let mut workspace_global_active: Vec<EcosystemId> = Vec::new();
    for eco in reg.all() {
        if !eco.is_workspace_global() { continue; }
        if is_transitive_only(&eco.activation()) { continue; }
        if evaluate_activation_scoped(
            &eco.activation(),
            eco.id(),
            &workspace_scope,
            &workspace_global_active,
        ) {
            workspace_global_active.push(eco.id());
        }
    }
    if !workspace_global_active.is_empty() {
        for actives in out.values_mut() {
            for id in &workspace_global_active {
                if !actives.contains(id) {
                    actives.push(*id);
                }
            }
        }
    }
    out
}

/// Build the workspace-wide `active_ecosystems` set as the union of every
/// package's per-package actives, preserving registry order so legacy
/// consumers see a stable shape.
fn union_per_package_actives(
    per_package: &HashMap<i64, Vec<EcosystemId>>,
    reg: &EcosystemRegistry,
) -> Vec<EcosystemId> {
    let mut seen: HashSet<EcosystemId> = HashSet::new();
    for actives in per_package.values() {
        for id in actives {
            seen.insert(*id);
        }
    }
    reg.all().iter().map(|e| e.id()).filter(|id| seen.contains(id)).collect()
}

/// Scoped view of the project (or one of its packages) that the
/// activation evaluator consults. Workspace-wide activation passes the
/// union manifests + workspace-wide language presence + project root;
/// per-package activation passes the package's own manifests, the
/// package's language presence, and the package's absolute path as the
/// glob root for `ManifestFieldContains`.
struct ActivationScope<'a> {
    manifests: &'a HashMap<ManifestKind, ManifestData>,
    language_presence: &'a HashSet<String>,
    glob_root: &'a Path,
}

fn evaluate_activation(
    act: &EcosystemActivation,
    eco_id: EcosystemId,
    ctx: &ProjectContext,
    already_active: &[EcosystemId],
) -> bool {
    let scope = ActivationScope {
        manifests: &ctx.manifests,
        language_presence: &ctx.language_presence,
        glob_root: &ctx.project_root,
    };
    evaluate_activation_scoped(act, eco_id, &scope, already_active)
}

fn evaluate_activation_scoped(
    act: &EcosystemActivation,
    eco_id: EcosystemId,
    scope: &ActivationScope<'_>,
    already_active: &[EcosystemId],
) -> bool {
    match act {
        EcosystemActivation::Always => true,
        EcosystemActivation::Never => false,
        EcosystemActivation::ManifestMatch => {
            ecosystem_manifest_present_scoped(eco_id, scope)
        }
        EcosystemActivation::LanguagePresent(lang) => {
            scope.language_presence.contains(*lang)
        }
        EcosystemActivation::ManifestFieldContains {
            manifest_glob,
            field_path,
            value,
        } => manifest_field_contains(scope.glob_root, manifest_glob, field_path, value),
        EcosystemActivation::AlwaysOnPlatform(plat) => matches_platform(*plat),
        EcosystemActivation::TransitiveOn(id) => already_active.contains(id),
        EcosystemActivation::All(clauses) => clauses
            .iter()
            .all(|c| evaluate_activation_scoped(c, eco_id, scope, already_active)),
        EcosystemActivation::Any(clauses) => clauses
            .iter()
            .any(|c| evaluate_activation_scoped(c, eco_id, scope, already_active)),
    }
}

/// `ManifestMatch` evaluation. Returns `true` iff at least one
/// `ManifestKind` claimed by the ecosystem is present in the scope's
/// manifest set. Ecosystems with no claimed kinds (returned empty from
/// `manifest_kinds_for_ecosystem`) always evaluate to `false`.
fn ecosystem_manifest_present_scoped(eco_id: EcosystemId, scope: &ActivationScope<'_>) -> bool {
    let kinds = manifest_kinds_for_ecosystem(eco_id);
    if kinds.is_empty() {
        return false;
    }
    kinds.iter().any(|k| scope.manifests.contains_key(k))
}

/// `ManifestFieldContains` evaluation. Walks `project_root` for files
/// matching `manifest_glob`, parses each as JSON or YAML based on
/// extension, traverses `field_path` (a `.`-separated chain of keys),
/// and returns `true` iff any matched file's resolved field contains
/// `value` — for arrays "contains" means membership, for scalars it
/// means string equality.
///
/// Glob support is intentionally narrow: `**/<basename>` matches a
/// file with that basename at any depth; a bare `<basename>` matches
/// only at the project root. Anything else falls through to literal
/// path equality. This covers every documented use case (tsconfig.json,
/// pubspec.yaml, project.godot, bicepconfig.json) without pulling in a
/// full glob crate.
fn manifest_field_contains(
    project_root: &Path,
    manifest_glob: &str,
    field_path: &str,
    value: &str,
) -> bool {
    if project_root.as_os_str().is_empty() {
        return false;
    }
    for path in find_manifests_matching(project_root, manifest_glob) {
        if manifest_file_field_contains(&path, field_path, value) {
            return true;
        }
    }
    false
}

fn find_manifests_matching(project_root: &Path, glob: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(basename) = glob.strip_prefix("**/") {
        // Walk the project tree; collect every file whose basename matches.
        // Reuses the same `ignore` walker the rest of the indexer uses so
        // gitignore + standard exclusions are respected.
        let walker = ignore::WalkBuilder::new(project_root)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .max_depth(Some(20))
            .build();
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            if entry.file_name().to_str() == Some(basename) {
                out.push(entry.path().to_path_buf());
            }
        }
    } else {
        // Bare basename → look at the project root only.
        let p = project_root.join(glob);
        if p.is_file() {
            out.push(p);
        }
    }
    out
}

fn manifest_file_field_contains(path: &Path, field_path: &str, value: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase());
    let parsed: Option<serde_json::Value> = match ext.as_deref() {
        Some("json") => serde_json::from_str(&text).ok(),
        Some("yaml") | Some("yml") => serde_yaml::from_str(&text).ok(),
        // Plain-text manifests (project.godot, .ini-shaped) — fall back to
        // a substring search keyed on `field_path = value` shape. Good
        // enough for the simple cases this evaluator targets; ecosystems
        // needing structured access for INI-shaped manifests should ship
        // a dedicated reader.
        _ => return text.contains(value),
    };
    let Some(root) = parsed else { return false };
    let target = traverse_field_path(&root, field_path);
    field_value_contains(target, value)
}

fn traverse_field_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut cur = root;
    for segment in path.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur)
}

fn field_value_contains(value: Option<&serde_json::Value>, needle: &str) -> bool {
    let Some(v) = value else { return false };
    match v {
        serde_json::Value::Array(items) => items.iter().any(|item| match item {
            serde_json::Value::String(s) => s.eq_ignore_ascii_case(needle),
            other => other.to_string().contains(needle),
        }),
        serde_json::Value::String(s) => s.eq_ignore_ascii_case(needle) || s.contains(needle),
        serde_json::Value::Object(map) => map.contains_key(needle),
        other => other.to_string().contains(needle),
    }
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
