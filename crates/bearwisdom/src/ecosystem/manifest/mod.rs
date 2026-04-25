// =============================================================================
// indexer/manifest/mod.rs — ManifestReader trait and dispatch
//
// Each language ecosystem has a dedicated reader that locates manifest files
// under the project root and extracts a normalized `ManifestData` from them.
//
// `read_all_manifests` runs every registered reader and returns a map keyed
// by `ManifestKind`. `build_project_context` calls this once and back-fills
// the per-language fields on `ProjectContext` for backward compatibility.
// =============================================================================

// NOTE: `cargo` manifest reader migrated to `crate::ecosystem::cargo` in
// Phase 2+3. Call sites now import directly from the ecosystem module.
// NOTE: `composer` manifest reader migrated to `crate::ecosystem::composer` in Phase 2+3.
// NOTE: `description` manifest reader migrated to `crate::ecosystem::cran` in Phase 2+3.
// NOTE: `gemfile` manifest reader migrated to `crate::ecosystem::rubygems` in Phase 2+3.
// NOTE: `go_mod` manifest reader migrated to `crate::ecosystem::go_mod` in Phase 2+3.
pub mod gradle;
pub mod js_config_aliases;
pub mod maven;
pub mod mix;
pub mod npm;
// NOTE: `nuget` manifest reader migrated to `crate::ecosystem::nuget` in Phase 2+3.
pub mod sbt;
// NOTE: `opam` manifest reader migrated to `crate::ecosystem::opam` in Phase 2+3.
pub mod gleam;
// NOTE: `zig_zon` manifest reader migrated to `crate::ecosystem::zig_pkg` in Phase 2+3.
pub mod clojure;
// NOTE: `rockspec` manifest reader migrated to `crate::ecosystem::luarocks` in Phase 2+3.
// NOTE: `pubspec` manifest reader migrated to `crate::ecosystem::pub_pkg` in Phase 2+3.
// NOTE: `pyproject` manifest reader migrated to `crate::ecosystem::pypi` in Phase 2+3.
// NOTE: `swift_pm` manifest reader migrated to `crate::ecosystem::spm` in Phase 2+3.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Normalized set of dependency names for a given ecosystem.
pub type DependencySet = HashSet<String>;

/// Which ecosystem produced a `ManifestData` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ManifestKind {
    Npm,
    Cargo,
    NuGet,
    GoMod,
    PyProject,
    Gradle,
    Maven,
    Gemfile,
    Composer,
    SwiftPM,
    Pubspec,
    Mix,
    Description,
    Sbt,
    Opam,
    Gleam,
    ZigZon,
    Clojure,
    Rockspec,
}

/// Normalized data extracted from a project manifest.
#[derive(Debug, Clone, Default)]
pub struct ManifestData {
    /// Dependency names (package/crate/gem/etc names, no versions).
    pub dependencies: DependencySet,
    /// Go: module path from `go.mod`. Swift: package name. Others: `None`.
    pub module_path: Option<String>,
    /// .NET: combined SDK implicit usings + `global using` statements.
    pub global_usings: Vec<String>,
    /// .NET: most capable SDK type string (`"web"`, `"base"`, etc.).
    pub sdk_type: Option<String>,
    /// .NET: workspace sibling projects referenced via
    /// `<ProjectReference Include="..."/>`. Each entry is the referenced
    /// project's filename stem (e.g. `../Shared/Shared.csproj` →
    /// `"Shared"`). Matches sibling packages' `declared_name`.
    pub project_refs: Vec<String>,
    /// TypeScript: `compilerOptions.paths` alias map from tsconfig.json.
    /// Each entry is `(alias_prefix, target_prefix)` with trailing `*`
    /// stripped — e.g. `("@/", "src/")` lets `@/utils` resolve to
    /// `src/utils`. Populated only for the NuGet-sibling TS ecosystem;
    /// other manifest kinds leave this empty.
    pub tsconfig_paths: Vec<(String, String)>,
    /// TypeScript: `compilerOptions.types` from tsconfig.json — the list
    /// of packages whose type definitions are auto-loaded as ambient
    /// globals (the same way TS itself treats them). Each entry is the
    /// raw package name as listed: `"vitest/globals"`, `"node"`,
    /// `"@types/jest"`, `"@playwright/test"`, etc. The resolver uses
    /// these to identify which external package files contribute
    /// ambient symbols (no `import` statement needed in user code) so
    /// bare-name refs like `expect`, `describe`, `process` prefer those
    /// candidates over identically-named symbols in non-ambient
    /// packages, even when ambiguity strikes. Empty when no tsconfig
    /// or no `types` field declared.
    pub tsconfig_types: Vec<String>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Reads one ecosystem's manifest files and returns normalized data.
///
/// Implementations are stateless — construct once, call `read` many times.
///
/// Two APIs:
///   - `read()` — legacy. Returns a single unioned `ManifestData` for the whole
///     project root. Kept so single-manifest readers (go.mod, Gemfile, etc.) need
///     no changes.
///   - `read_all()` — per-file. Returns one entry per manifest file found. Monorepo
///     readers (npm, cargo, gradle, maven, pubspec, mix, nuget, composer) override
///     this; single-manifest readers use the default impl which wraps `read()`.
///
/// The top-level `read_all_manifests_per_package` walks every reader via `read_all`
/// and builds a flat `Vec<PackageManifest>` for per-package resolution work (M2+).
pub trait ManifestReader: Send + Sync {
    fn kind(&self) -> ManifestKind;

    /// Scan `project_root` for manifest files.
    ///
    /// Returns `None` when no manifest for this ecosystem is found.
    /// Returns `Some(ManifestData::default())` when a file exists but is empty
    /// or unparseable — callers can still detect the ecosystem's presence.
    fn read(&self, project_root: &Path) -> Option<ManifestData>;

    /// Scan `project_root` for EVERY manifest file of this kind and return one
    /// entry per file. Each entry is `(package_dir, manifest_path, data, name)`:
    ///
    ///   - `package_dir` — absolute path to the directory containing the manifest
    ///   - `manifest_path` — absolute path to the manifest file itself
    ///   - `data` — parsed `ManifestData` for **that file alone** (NOT unioned)
    ///   - `name` — package name extracted from the manifest (e.g. `package.json`'s
    ///     `"name"` field, or `[package].name` in Cargo.toml). `None` when the
    ///     manifest has no name field (most single-file manifests).
    ///
    /// Default impl calls `read()` once at the root and returns a single entry.
    /// Monorepo-aware readers override this.
    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let Some(data) = self.read(project_root) else { return Vec::new() };
        vec![ReaderEntry {
            package_dir: project_root.to_path_buf(),
            manifest_path: project_root.to_path_buf(),
            data,
            name: None,
        }]
    }
}

/// One manifest file's contribution as produced by a `ManifestReader`.
#[derive(Debug, Clone)]
pub struct ReaderEntry {
    pub package_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub data: ManifestData,
    pub name: Option<String>,
}

/// Per-package manifest data — one entry per detected workspace package.
///
/// `path` is **relative** to `project_root` (matches how `packages.path` is
/// stored in the DB). A single-package project yields one `PackageManifest`
/// with `path == PathBuf::new()`.
#[derive(Debug, Clone)]
pub struct PackageManifest {
    /// Package name extracted from the manifest. Falls back to the directory
    /// name when the manifest has no name field.
    pub name: String,
    /// Relative path from `project_root` to the package directory.
    pub path: PathBuf,
    /// Which ecosystem this manifest belongs to.
    pub kind: ManifestKind,
    /// Dependencies declared by THIS package only — NOT merged with siblings.
    pub data: ManifestData,
    /// Absolute path to the manifest file that produced this data.
    pub manifest_path: PathBuf,
}

// ---------------------------------------------------------------------------
// Registry & dispatch
// ---------------------------------------------------------------------------

/// All registered `ManifestReader` implementations, in stable order.
fn all_readers() -> Vec<Box<dyn ManifestReader>> {
    vec![
        Box::new(npm::NpmManifest),
        Box::new(crate::ecosystem::cargo::CargoManifest),
        Box::new(crate::ecosystem::nuget::NuGetManifest),
        Box::new(crate::ecosystem::go_mod::GoModManifest),
        Box::new(crate::ecosystem::pypi::PyProjectManifest),
        Box::new(gradle::GradleManifest),
        Box::new(maven::MavenManifest),
        Box::new(crate::ecosystem::rubygems::GemfileManifest),
        Box::new(crate::ecosystem::composer::ComposerManifest),
        Box::new(crate::ecosystem::spm::SwiftPMManifest),
        Box::new(crate::ecosystem::pub_pkg::PubspecManifest),
        Box::new(mix::MixManifest),
        Box::new(sbt::SbtManifest),
        Box::new(crate::ecosystem::opam::OpamManifest),
        Box::new(gleam::GleamManifest),
        Box::new(crate::ecosystem::zig_pkg::ZigZonManifest),
        Box::new(clojure::ClojureManifest),
        Box::new(crate::ecosystem::luarocks::RockspecManifest),
        Box::new(crate::ecosystem::cran::DescriptionManifest),
    ]
}

/// Run every registered reader against `project_root` and union each
/// ecosystem's results into a single `ManifestData` per kind.
///
/// This is the **legacy** API. It is the union of every manifest found across
/// the project — fine for whole-project classification but incorrect for
/// per-package resolution. Per-package work calls
/// `read_all_manifests_per_package` directly.
///
/// Returns only the ecosystems for which a manifest was actually found.
pub fn read_all_manifests(project_root: &Path) -> HashMap<ManifestKind, ManifestData> {
    let per_package = read_all_manifests_per_package(project_root);
    let mut result: HashMap<ManifestKind, ManifestData> = HashMap::new();
    for pm in per_package {
        let entry = result.entry(pm.kind).or_default();
        entry.dependencies.extend(pm.data.dependencies);
        // module_path: last non-None wins. Legacy behavior is undefined when
        // multiple packages declare one (never happened pre-M1 — only go.mod
        // and Swift PM set this, both single-manifest), so last-write is safe.
        if pm.data.module_path.is_some() {
            entry.module_path = pm.data.module_path;
        }
        entry.global_usings.extend(pm.data.global_usings);
        if pm.data.sdk_type.is_some() {
            entry.sdk_type = pm.data.sdk_type;
        }
        // tsconfig-style aliases (from tsconfig.json + vite/vue/webpack
        // configs) must propagate into the union or single-package projects
        // — those that don't trigger the per-package builder — will never
        // see any alias rewrite. Deduplicate so repeat runs stay idempotent.
        for alias in pm.data.tsconfig_paths {
            if !entry.tsconfig_paths.contains(&alias) {
                entry.tsconfig_paths.push(alias);
            }
        }
        for pr in pm.data.project_refs {
            if !entry.project_refs.contains(&pr) {
                entry.project_refs.push(pr);
            }
        }
    }
    result
}

/// Run every registered reader against `project_root` and return one
/// `PackageManifest` per manifest file found.
///
/// For a single-package project (no workspace), returns one entry per ecosystem
/// with `path == PathBuf::new()`.
///
/// For a monorepo (pnpm, Cargo workspace, etc.), returns one entry per member —
/// each entry's `data` reflects **only** that package's declarations, with no
/// cross-package pollution.
pub fn read_all_manifests_per_package(project_root: &Path) -> Vec<PackageManifest> {
    let mut out = Vec::new();
    for reader in all_readers() {
        let kind = reader.kind();
        for entry in reader.read_all(project_root) {
            let rel_path = entry
                .package_dir
                .strip_prefix(project_root)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| PathBuf::new());
            let name = entry.name.unwrap_or_else(|| {
                rel_path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| {
                        project_root
                            .file_name()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "root".to_string())
                    })
            });
            out.push(PackageManifest {
                name,
                path: rel_path,
                kind,
                data: entry.data,
                manifest_path: entry.manifest_path,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn names_by_kind<'a>(
        manifests: &'a [PackageManifest],
        kind: ManifestKind,
    ) -> Vec<&'a str> {
        let mut names: Vec<&str> = manifests
            .iter()
            .filter(|m| m.kind == kind)
            .map(|m| m.name.as_str())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn per_package_splits_monorepo_npm() {
        // Synthetic pnpm-style monorepo: 3 packages, each with its own deps.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(root, "package.json", r#"{"name":"root","private":true}"#);
        write_file(
            root,
            "packages/server/package.json",
            r#"{"name":"@app/server","dependencies":{"express":"4"}}"#,
        );
        write_file(
            root,
            "packages/web/package.json",
            r#"{"name":"@app/web","dependencies":{"react":"18"}}"#,
        );
        write_file(
            root,
            "packages/e2e/package.json",
            r#"{"name":"@app/e2e","devDependencies":{"playwright":"1"}}"#,
        );

        let per_pkg = read_all_manifests_per_package(root);
        let npm: Vec<&PackageManifest> =
            per_pkg.iter().filter(|m| m.kind == ManifestKind::Npm).collect();

        // 4 package.json files → 4 entries (root counts).
        assert_eq!(npm.len(), 4, "expected 4 npm manifests, got {}", npm.len());

        let names = names_by_kind(&per_pkg, ManifestKind::Npm);
        assert_eq!(names, vec!["@app/e2e", "@app/server", "@app/web", "root"]);

        // Per-package dep isolation: server has express but NOT react or playwright.
        let server = npm.iter().find(|m| m.name == "@app/server").unwrap();
        assert!(server.data.dependencies.contains("express"));
        assert!(!server.data.dependencies.contains("react"));
        assert!(!server.data.dependencies.contains("playwright"));

        let web = npm.iter().find(|m| m.name == "@app/web").unwrap();
        assert!(web.data.dependencies.contains("react"));
        assert!(!web.data.dependencies.contains("express"));

        let e2e = npm.iter().find(|m| m.name == "@app/e2e").unwrap();
        assert!(e2e.data.dependencies.contains("playwright"));
        assert!(!e2e.data.dependencies.contains("react"));
    }

    #[test]
    fn per_package_splits_cargo_workspace() {
        // Synthetic Cargo workspace with 2 member crates.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "Cargo.toml",
            "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\n",
        );
        write_file(
            root,
            "crates/a/Cargo.toml",
            "[package]\nname = \"crate-a\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
        );
        write_file(
            root,
            "crates/b/Cargo.toml",
            "[package]\nname = \"crate-b\"\nversion = \"0.1.0\"\n\n[dependencies]\ntokio = \"1\"\n",
        );

        let per_pkg = read_all_manifests_per_package(root);
        let cargo: Vec<&PackageManifest> = per_pkg
            .iter()
            .filter(|m| m.kind == ManifestKind::Cargo)
            .collect();

        // 3 Cargo.toml files (workspace root + 2 members).
        assert_eq!(cargo.len(), 3);

        let names = names_by_kind(&per_pkg, ManifestKind::Cargo);
        // Workspace root has no [package].name → falls back to dir name ("the temp dir's last segment").
        // We can't predict the temp dir's name, but crate-a and crate-b must be present.
        assert!(names.contains(&"crate-a"));
        assert!(names.contains(&"crate-b"));

        let a = cargo.iter().find(|m| m.name == "crate-a").unwrap();
        assert!(a.data.dependencies.contains("serde"));
        assert!(!a.data.dependencies.contains("tokio"));

        let b = cargo.iter().find(|m| m.name == "crate-b").unwrap();
        assert!(b.data.dependencies.contains("tokio"));
        assert!(!b.data.dependencies.contains("serde"));
    }

    #[test]
    fn single_package_yields_one_entry_per_ecosystem() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "package.json",
            r#"{"name":"solo","dependencies":{"lodash":"4"}}"#,
        );

        let per_pkg = read_all_manifests_per_package(root);
        let npm: Vec<&PackageManifest> =
            per_pkg.iter().filter(|m| m.kind == ManifestKind::Npm).collect();

        assert_eq!(npm.len(), 1);
        assert_eq!(npm[0].name, "solo");
        assert_eq!(npm[0].path, PathBuf::new());
        assert!(npm[0].data.dependencies.contains("lodash"));
    }

    #[test]
    fn legacy_read_all_matches_union_of_per_package() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "packages/a/package.json",
            r#"{"name":"a","dependencies":{"foo":"1"}}"#,
        );
        write_file(
            root,
            "packages/b/package.json",
            r#"{"name":"b","dependencies":{"bar":"1"}}"#,
        );

        let legacy = read_all_manifests(root);
        let npm_data = legacy.get(&ManifestKind::Npm).expect("npm union present");

        // Union must contain both foo and bar.
        assert!(npm_data.dependencies.contains("foo"));
        assert!(npm_data.dependencies.contains("bar"));
    }

    #[test]
    fn empty_project_returns_nothing() {
        let tmp = TempDir::new().unwrap();
        let per_pkg = read_all_manifests_per_package(tmp.path());
        assert!(per_pkg.is_empty());

        let legacy = read_all_manifests(tmp.path());
        assert!(legacy.is_empty());
    }

    #[test]
    fn skips_node_modules_when_walking() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "package.json",
            r#"{"name":"host","dependencies":{"react":"18"}}"#,
        );
        // A sub-package.json inside node_modules must NOT be picked up.
        write_file(
            root,
            "node_modules/react/package.json",
            r#"{"name":"react","version":"18.0.0","dependencies":{"loose-envify":"1"}}"#,
        );

        let per_pkg = read_all_manifests_per_package(root);
        let npm: Vec<&PackageManifest> =
            per_pkg.iter().filter(|m| m.kind == ManifestKind::Npm).collect();
        assert_eq!(npm.len(), 1);
        assert_eq!(npm[0].name, "host");
    }

    #[test]
    fn package_path_is_relative_to_project_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "packages/server/package.json",
            r#"{"name":"server","dependencies":{}}"#,
        );

        let per_pkg = read_all_manifests_per_package(root);
        let server = per_pkg
            .iter()
            .find(|m| m.name == "server")
            .expect("server manifest");

        assert_eq!(server.path, PathBuf::from("packages").join("server"));
    }

    #[test]
    fn manifest_path_is_absolute() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write_file(
            root,
            "package.json",
            r#"{"name":"solo","dependencies":{}}"#,
        );

        let per_pkg = read_all_manifests_per_package(root);
        let solo = per_pkg.iter().find(|m| m.name == "solo").unwrap();
        assert!(solo.manifest_path.is_absolute());
        assert!(solo.manifest_path.ends_with("package.json"));
    }
}
