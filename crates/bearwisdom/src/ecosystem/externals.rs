// =============================================================================
// indexer/externals/mod.rs — external dependency source discovery + walking
//
// Finds the on-disk root of each external dependency declared in a project's
// manifest and enumerates the source files under it. Indexed rows produced
// from these files are written with `origin='external'`, so user-facing
// queries can filter them out while the resolver can still find them.
//
// Ecosystems are split into per-file submodules. Shared types and utilities
// live here in mod.rs; re-exports maintain the same public API as the
// previous single-file layout.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::types::ParsedFile;
use crate::walker::WalkedFile;
use tracing::debug;

// NOTE: all per-language locators have migrated to `crate::ecosystem::*` in
// Phase 2+3. This module now only holds the `ExternalSourceLocator` trait,
// `ExternalDepRoot` struct, shared Maven helpers, and legacy re-exports
// used by callers still on the old import paths.

// Re-exports of functions that moved to ecosystem modules, for back-compat.
pub use crate::ecosystem::nuget::{parse_dotnet_externals, nuget_packages_root};

/// A discovered external dependency root — the directory containing one
/// version of one package on disk.
#[derive(Debug, Clone)]
pub struct ExternalDepRoot {
    /// Canonical module path (e.g., "github.com/gin-gonic/gin").
    pub module_path: String,
    /// Semantic version string as it appears in go.mod (e.g., "v1.9.1").
    pub version: String,
    /// Absolute path to the module cache directory on disk.
    pub root: PathBuf,
    /// Ecosystem identifier. "go" for now.
    pub ecosystem: &'static str,
    /// M3: which workspace package declared this dep. `None` for
    /// single-project layouts or when the orchestrator hasn't stamped
    /// attribution yet. Stamped per-package by
    /// `ExternalSourceLocator::locate_roots_for_package`'s default impl
    /// (and overrides) and read by `parse_external_sources` to populate
    /// the `package_deps` table and attribute shared walks to multiple
    /// declaring packages.
    pub package_id: Option<i64>,
    /// R3: the exact import specifiers from user code that drove this dep
    /// onto the discovery result. Populated by ecosystems that can narrow
    /// `resolve_import` walking by user-observed demand — e.g., Go's
    /// import paths (`"github.com/gin-gonic/gin/binding"`) or Java's
    /// fully-qualified class imports. Empty vec means "no demand data;
    /// walk everything the module exposes". Other ecosystems leave it
    /// empty.
    pub requested_imports: Vec<String>,
}

// ---------------------------------------------------------------------------
// ExternalSourceLocator — per-ecosystem external discovery trait
// ---------------------------------------------------------------------------

/// One-language strategy for finding external dependency source on disk and
/// turning it into walker / parser input for the main indexing pipeline.
///
/// Two output shapes, chosen per ecosystem:
///
///   * **Source-file locators** implement `locate_roots` + `walk_root`. The
///     pipeline walks each root into `WalkedFile`s and parses them with the
///     language's extractor. Used by Go, Python, TypeScript, Java (sources
///     jar extraction produces real .java files on disk), and any future
///     source-shipping ecosystem (Ruby, Elixir, Dart, Rust, PHP, Scala,
///     Lua, OCaml, Perl, etc.).
///
///   * **Metadata locators** implement `parse_metadata_only`. The pipeline
///     trusts the returned `ParsedFile` entries without re-walking. Used by
///     ecosystems where source isn't distributed — today only .NET (DLL
///     metadata via dotscope). Haskell `.hi`, OCaml `.cmi`, R `.rdb` are
///     future consumers of this path.
///
/// Implementations may return both kinds at once (future: Java could return
/// source jars when available and `.class` metadata as a fallback) — the
/// default trait methods make unused paths zero-cost.
pub trait ExternalSourceLocator: Send + Sync {
    /// Stable identifier for this locator. Used in logs and diagnostics.
    /// Must be distinct per ecosystem: `"go"`, `"python"`, `"typescript"`,
    /// `"java"`, `"dotnet"`, `"ruby"`, `"elixir"`, etc.
    fn ecosystem(&self) -> &'static str;

    /// Discover every external package root belonging to this ecosystem
    /// for the given project. An empty vec means "nothing to index" —
    /// never an error. Missing package caches, unavailable toolchains,
    /// and absent manifests all degrade to an empty vec.
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        Vec::new()
    }

    /// M3: Discover external package roots scoped to a single workspace
    /// package. Called once per `(locator, package)` pair by the full-index
    /// orchestrator when a monorepo is detected. Default implementation
    /// delegates to `locate_roots(package_abs_path)` and stamps the
    /// package id on every returned root.
    ///
    /// Ecosystems that need workspace-aware behavior — TypeScript walking
    /// up ancestors to find a hoisted `node_modules`, Python checking
    /// a parent venv, .NET reading one csproj — override this method.
    ///
    /// The `workspace_root` is the full project root (useful for upward
    /// ancestor walks), the `package_abs_path` is the absolute path of
    /// the workspace package's own directory.
    fn locate_roots_for_package(
        &self,
        _workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        let mut roots = self.locate_roots(package_abs_path);
        for r in &mut roots {
            r.package_id = Some(package_id);
        }
        roots
    }

    /// Enumerate source files under one discovered root. Language-specific
    /// filtering (skip tests, skip docs, skip minified bundles) lives here.
    /// Only called for roots this locator's `locate_roots` returned.
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    /// Alternative output path for ecosystems where source isn't on disk.
    /// Returns pre-built `ParsedFile` rows straight from compiled metadata
    /// (.NET DLL via dotscope today, Haskell .hi / OCaml .cmi / R .rdb in
    /// future phases). Default implementation returns `None`, meaning this
    /// locator uses the source walk path exclusively.
    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        None
    }

    /// Optional per-file post-processing hook applied after the main
    /// extractor has parsed a walked file. Used by the TS locator today
    /// to prefix bare declaration symbols with their package name so the
    /// Tier-1 resolver can match `package.Symbol` lookups. Default is a
    /// no-op.
    fn post_process_parsed(&self, _parsed: &mut ParsedFile) {}
}

/// Extract the package name from a TS external-file virtual path like
/// `ext:ts:@types/react/index.d.ts` → `@types/react`, or
/// `ext:ts:lodash/lodash.d.ts` → `lodash`. Used by the TS locator's
/// `post_process_parsed` hook to prefix bare declaration-file symbols with
/// their owning package name so the Tier-1 resolver matches
/// `import { X } from 'pkg'` → `pkg.X`.
///
/// This is the canonical implementation — previously lived as a private
/// helper in `indexer/full.rs`. Moved here alongside the TS locator so the
/// Phase 0 refactor keeps the helper and its one caller in the same file.
pub(crate) fn ts_package_from_virtual_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("ext:ts:")?;
    // Scoped package: `@foo/bar/...` — the package name is the first two
    // slash-separated segments joined.
    if rest.starts_with('@') {
        let mut parts = rest.splitn(3, '/');
        let scope = parts.next()?;
        let name = parts.next()?;
        let end_byte = scope.len() + 1 + name.len();
        Some(&rest[..end_byte])
    } else {
        let slash = rest.find('/')?;
        Some(&rest[..slash])
    }
}

/// Convenience — build the fixed set of 5 locators that ship today. Post-
/// Phase 4 the authoritative dispatch path is
/// `ecosystem::default_registry()`; this standalone builder stays available
/// for unit tests and diagnostic commands that want a direct handle on a
/// known locator without iterating the registry.
pub fn builtin_locators() -> Vec<Arc<dyn ExternalSourceLocator>> {
    vec![
        Arc::new(crate::ecosystem::GoModEcosystem),
        Arc::new(crate::ecosystem::PypiEcosystem),
        Arc::new(crate::ecosystem::NpmEcosystem),
        Arc::new(crate::ecosystem::MavenEcosystem),
        Arc::new(crate::ecosystem::NugetEcosystem),
    ]
}

// ---------------------------------------------------------------------------
// Shared Maven utilities (used by Java, Scala, Clojure)
// ---------------------------------------------------------------------------

/// Locate `$MAVEN_LOCAL_REPO` in the order BEARWISDOM_JAVA_MAVEN_REPO →
/// `$HOME/.m2/repository` → `$USERPROFILE/.m2/repository`. Returns `None`
/// when no directory is found — Java externals silently drop.
/// Pick the highest-precedence version from a list of version strings —
/// semver-aware, so `3.12.0` is correctly chosen over `3.9.1` (lexicographic
/// sort gives the wrong answer because "9" > "1" string-wise).
///
/// Comparison rule: split each version into (numeric_components, qualifier).
/// Numeric components are compared numerically left-to-right. Qualifiers
/// (e.g., `-RC1`, `-M43`, `-SNAPSHOT`) sort lexicographically AFTER the
/// numeric prefix and AFTER no-qualifier — so `1.0.0` > `1.0.0-RC1`, which
/// matches Maven / sbt convention. When a version doesn't parse as semver
/// at all (e.g., a date-stamp), it falls to lexicographic ordering.
pub(crate) fn pick_newest_version(versions: &[String]) -> Option<String> {
    versions
        .iter()
        .max_by(|a, b| version_compare(a, b))
        .cloned()
}

fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let (a_nums, a_qual) = split_version(a);
    let (b_nums, b_qual) = split_version(b);
    for i in 0..a_nums.len().max(b_nums.len()) {
        let av = a_nums.get(i).copied().unwrap_or(0);
        let bv = b_nums.get(i).copied().unwrap_or(0);
        match av.cmp(&bv) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    // Equal numeric parts: no-qualifier > has-qualifier (release beats pre-release).
    match (a_qual.is_empty(), b_qual.is_empty()) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Greater,
        (false, true) => std::cmp::Ordering::Less,
        (false, false) => a_qual.cmp(&b_qual),
    }
}

fn split_version(v: &str) -> (Vec<u64>, String) {
    let bytes = v.as_bytes();
    let mut nums = Vec::new();
    let mut i = 0;
    let len = bytes.len();
    while i < len {
        let mut end = i;
        while end < len && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end == i {
            break;
        }
        if let Ok(n) = v[i..end].parse::<u64>() {
            nums.push(n);
        }
        i = end;
        // Step over the dot separator between numeric components; anything
        // else (`-`, letter) marks the qualifier boundary.
        if i < len && bytes[i] == b'.' {
            i += 1;
            continue;
        }
        break;
    }
    let qualifier = v[i..].to_string();
    (nums, qualifier)
}

pub fn maven_local_repo() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_JAVA_MAVEN_REPO") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home).join(".m2").join("repository");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Resolve `{repo}/{groupId/as/path}/{artifactId}/{version}/` for a coord.
/// When `coord.version` is None, fall back to the lexicographically largest
/// subdirectory of `{repo}/{group}/{artifact}/` so Spring Boot starters
/// that resolve `${spring.version}` still match whatever is locally cached.
/// Returns `(resolved_version, artifact_dir)`.
pub(crate) fn resolve_maven_artifact_dir(
    repo: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<(String, PathBuf)> {
    let mut group_path = repo.to_path_buf();
    for seg in coord.group_id.split('.') {
        group_path.push(seg);
    }
    group_path.push(&coord.artifact_id);
    if !group_path.is_dir() {
        return None;
    }

    let version = if let Some(v) = &coord.version {
        v.clone()
    } else {
        let entries = std::fs::read_dir(&group_path).ok()?;
        let versions: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                if e.file_type().ok()?.is_dir() {
                    e.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        pick_newest_version(&versions)?
    };

    let artifact_dir = group_path.join(&version);
    if artifact_dir.is_dir() {
        Some((version, artifact_dir))
    } else {
        None
    }
}

/// Locate `~/.gradle/caches/modules-2/files-2.1` — the Gradle dependency
/// cache. Layout: `<root>/<group>/<artifact>/<version>/<hash>/<file>` where
/// each `<hash>` directory holds exactly one artifact (the sha1 of the file).
/// Sources jars live alongside binary jars but are only downloaded when an
/// IDE or `--write-locks` request triggers them — this is a dev-machine
/// prerequisite, not BW's concern.
pub fn gradle_caches_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GRADLE_CACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let candidate = PathBuf::from(home)
        .join(".gradle")
        .join("caches")
        .join("modules-2")
        .join("files-2.1");
    if candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

/// Resolve a `MavenCoord` against the Gradle cache layout. Walks each
/// `<hash>` subdirectory under the version dir and returns the first
/// `<artifact>-<version>-sources.jar` that exists. When `coord.version`
/// is None, falls back to the lexicographically-largest version directory
/// just like `resolve_maven_artifact_dir`.
///
/// Returns `(resolved_version, sources_jar_path)` on success.
pub(crate) fn resolve_gradle_sources_jar(
    cache_root: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<(String, PathBuf)> {
    let group_dir = cache_root.join(&coord.group_id);
    let artifact_dir = group_dir.join(&coord.artifact_id);
    if !artifact_dir.is_dir() {
        return None;
    }

    let version = if let Some(v) = &coord.version {
        v.clone()
    } else {
        let versions: Vec<String> = std::fs::read_dir(&artifact_dir)
            .ok()?
            .flatten()
            .filter_map(|e| {
                if e.file_type().ok()?.is_dir() {
                    e.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        pick_newest_version(&versions)?
    };

    let version_dir = artifact_dir.join(&version);
    if !version_dir.is_dir() {
        return None;
    }
    let target_name = format!("{}-{}-sources.jar", coord.artifact_id, version);
    for entry in std::fs::read_dir(&version_dir).ok()?.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let candidate = p.join(&target_name);
        if candidate.is_file() {
            return Some((version, candidate));
        }
    }
    None
}

/// Locate the Coursier cache root. SBT-driven Scala projects (and any
/// Coursier-based JVM tool) populate this when the user runs
/// `sbt updateClassifiers` or `cs fetch --classifier sources`. Layout
/// under the cache is `<host>/<repo-path>/<group-as-path>/<artifact>/<version>/`
/// — i.e. the same Maven layout as `~/.m2/repository`, just rooted under
/// `<cache>/v1/https/<repo-host>/<repo-base>/`.
///
/// On Windows the cache lives at `%LOCALAPPDATA%/Coursier/Cache/v1`.
/// On macOS it's `~/Library/Caches/Coursier/v1`.
/// On Linux it's `~/.cache/coursier/v1` or `$XDG_CACHE_HOME/coursier/v1`.
pub fn coursier_cache_root() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_COURSIER_CACHE") {
        let p = PathBuf::from(explicit);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(dir) = std::env::var_os("COURSIER_CACHE") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }

    let candidates: Vec<PathBuf> = if cfg!(target_os = "windows") {
        let mut v = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(local).join("Coursier").join("Cache").join("v1"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            v.push(
                PathBuf::from(home)
                    .join("AppData")
                    .join("Local")
                    .join("Coursier")
                    .join("Cache")
                    .join("v1"),
            );
        }
        v
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        vec![PathBuf::from(home)
            .join("Library")
            .join("Caches")
            .join("Coursier")
            .join("v1")]
    } else {
        let home = std::env::var_os("HOME")?;
        let mut v = Vec::new();
        if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME") {
            v.push(PathBuf::from(xdg).join("coursier").join("v1"));
        }
        v.push(PathBuf::from(home).join(".cache").join("coursier").join("v1"));
        v
    };
    candidates.into_iter().find(|p| p.is_dir())
}

/// Resolve a `MavenCoord` against the Coursier cache. Walks the
/// `https/<host>/maven2/...` subtree (or any other repo root present)
/// looking for `<group-path>/<artifact>/<version>/<artifact>-<version>-sources.jar`.
/// Multiple repo hosts (Maven Central, Sonatype, custom) get probed in
/// directory iteration order; the first hit wins.
pub(crate) fn resolve_coursier_sources_jar(
    cache_root: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<(String, PathBuf)> {
    // Cache layout: <cache>/<scheme>/<host>/<repo-base>/<group-as-path>/<artifact>/<version>/
    // e.g.            v1/https/repo1.maven.org/maven2/co/fs2/fs2-core_3/3.12.0/
    // We don't enumerate scheme/host directories — Coursier nests them
    // multiple levels deep and the exact intermediary depends on the
    // repository. Recursive search bounded to a small depth, scoped to
    // directories named after the artifact's first group segment, is
    // both correct and cheap.
    let group_first = coord.group_id.split('.').next()?;
    let target_name = match &coord.version {
        Some(v) => format!("{}-{}-sources.jar", coord.artifact_id, v),
        None => String::new(),
    };

    fn search(
        dir: &Path,
        coord: &crate::ecosystem::manifest::maven::MavenCoord,
        group_first: &str,
        target_name: &str,
        depth: u32,
    ) -> Option<(String, PathBuf)> {
        if depth > 8 {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // Direct hit: we found the artifact's group-first directory.
            // Walk to <group-rest>/<artifact>/<version>/<artifact>-<version>-sources.jar.
            if name == group_first {
                let mut group_path = path.clone();
                let segments: Vec<&str> = coord.group_id.split('.').skip(1).collect();
                for seg in segments {
                    group_path.push(seg);
                }
                group_path.push(&coord.artifact_id);
                if !group_path.is_dir() {
                    continue;
                }
                let version = if let Some(v) = &coord.version {
                    v.clone()
                } else {
                    let versions: Vec<String> = std::fs::read_dir(&group_path)
                        .ok()?
                        .flatten()
                        .filter_map(|e| {
                            if e.file_type().ok()?.is_dir() {
                                e.file_name().into_string().ok()
                            } else {
                                None
                            }
                        })
                        .collect();
                    pick_newest_version(&versions)?
                };
                let jar = group_path.join(&version).join(format!(
                    "{}-{}-sources.jar",
                    coord.artifact_id, version
                ));
                if jar.is_file() {
                    return Some((version, jar));
                }
                continue;
            }
            // Otherwise descend through schema/host/repo-base wrappers.
            if let Some(hit) = search(&path, coord, group_first, target_name, depth + 1) {
                return Some(hit);
            }
        }
        None
    }

    search(cache_root, coord, group_first, &target_name, 0)
}

/// Scan a Coursier group directory for sub-module sources jars whose
/// artifact name begins with `artifact_prefix`. Returns
/// `(artifact_id, version, sources_jar_path)` for every sub-module jar
/// found. Intended for Scala aggregator artifacts (e.g. `scalatest_2.13`)
/// whose published `-sources.jar` contains only `META-INF` while the real
/// source files live in constituent modules (`scalatest-core_2.13`,
/// `scalatest-shouldmatchers_2.13`, etc.) under the same Coursier group dir.
///
/// `artifact_prefix` is the base name without the `_2.13` / `_3` Scala
/// version suffix and without any `-<module>` suffix — e.g. `"scalatest"`.
/// Every sibling artifact directory whose name starts with that prefix is
/// probed for a `-sources.jar` at `preferred_version`; when that version is
/// absent the newest available version is used instead.
pub(crate) fn resolve_coursier_submodule_jars(
    cache_root: &Path,
    group_id: &str,
    artifact_prefix: &str,
    preferred_version: Option<&str>,
) -> Vec<(String, String, PathBuf)> {
    // Walk scheme/host/repo-base wrappers to locate the Coursier group dir.
    fn find_group_dir(dir: &Path, group_id: &str, depth: u32) -> Option<PathBuf> {
        if depth > 8 { return None; }
        let group_first = group_id.split('.').next()?;
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if name == group_first {
                // Navigate remaining group segments.
                let mut group_path = path.clone();
                for seg in group_id.split('.').skip(1) {
                    group_path.push(seg);
                }
                if group_path.is_dir() {
                    return Some(group_path);
                }
            }
            if let Some(found) = find_group_dir(&path, group_id, depth + 1) {
                return Some(found);
            }
        }
        None
    }

    let Some(group_dir) = find_group_dir(cache_root, group_id, 0) else {
        return Vec::new();
    };

    let Ok(entries) = std::fs::read_dir(&group_dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let Some(artifact_dir_name) = path.file_name().and_then(|n| n.to_str()) else { continue };

        // Strip the Scala version suffix to get the base artifact name, then
        // check that it starts with our prefix. `scalatest-core_2.13` →
        // base = `scalatest-core`; `scalatest_2.13` itself is the aggregator
        // we already processed — skip it.
        let base = strip_scala_suffix(artifact_dir_name);
        if !base.starts_with(artifact_prefix) { continue; }
        // Skip the aggregator itself (exact match after suffix strip).
        if base == artifact_prefix { continue; }

        // Pick version: preferred first, then newest available.
        let version = if let Some(v) = preferred_version {
            let vdir = path.join(v);
            if vdir.is_dir() { v.to_string() }
            else {
                let Some(newest) = pick_newest_version_from_dir(&path) else { continue };
                newest
            }
        } else {
            let Some(newest) = pick_newest_version_from_dir(&path) else { continue };
            newest
        };

        let sources_jar = path.join(&version)
            .join(format!("{artifact_dir_name}-{version}-sources.jar"));
        if sources_jar.is_file() {
            out.push((artifact_dir_name.to_string(), version, sources_jar));
        }
    }
    out
}

/// Strip a Scala binary-version suffix from an artifact directory name.
/// `scalatest-core_2.13` → `scalatest-core`
/// `scalatest_3`         → `scalatest`
/// `scalatest-compatible` → `scalatest-compatible` (no suffix — unchanged)
pub(crate) fn strip_scala_suffix(name: &str) -> &str {
    for suffix in &["_2.13", "_2.12", "_2.11", "_3"] {
        if let Some(base) = name.strip_suffix(suffix) {
            return base;
        }
    }
    name
}

fn pick_newest_version_from_dir(dir: &Path) -> Option<String> {
    let versions: Vec<String> = std::fs::read_dir(dir).ok()?
        .flatten()
        .filter_map(|e| {
            if e.file_type().ok()?.is_dir() {
                e.file_name().into_string().ok()
            } else {
                None
            }
        })
        .collect();
    pick_newest_version(&versions)
}

/// Mini walker that finds every `pom.xml` under a project root up to a
/// bounded depth. Mirrors the helper in `manifest/maven.rs` because that
/// one is private to the module.
pub(crate) fn collect_pom_files_bounded(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if matches!(
                        name,
                        ".git" | "target" | "build" | "node_modules"
                            | ".gradle" | "bin" | "obj" | ".idea"
                    ) {
                        continue;
                    }
                }
                collect_pom_files_bounded(&path, out, depth + 1);
            } else if ft.is_file() {
                if path.file_name().and_then(|n| n.to_str()) == Some("pom.xml") {
                    out.push(path);
                }
            }
        }
    }
}

/// Compare the sources jar mtime against the newest `.java` file under
/// `cache_dir`. If the jar was updated more recently, the cache is stale
/// and callers should re-extract.
pub(crate) fn is_cache_stale(jar: &Path, cache_dir: &Path) -> bool {
    let jar_mtime = match std::fs::metadata(jar).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let entries = match std::fs::read_dir(cache_dir) {
        Ok(e) => e,
        Err(_) => return true,
    };
    let mut newest: Option<std::time::SystemTime> = None;
    for entry in entries.flatten() {
        if let Ok(md) = entry.metadata() {
            if let Ok(t) = md.modified() {
                newest = Some(newest.map(|cur| cur.max(t)).unwrap_or(t));
            }
        }
    }
    match newest {
        Some(t) => jar_mtime > t,
        None => true,
    }
}

/// Extract all `.java` entries from a Maven `-sources.jar` into `dest`.
/// Skips entries whose path traverses out of `dest` (zip-slip guard) and
/// ignores non-`.java` files (META-INF, pom.properties, etc.).
pub(crate) fn extract_java_sources_jar(jar_path: &Path, dest: &Path) -> std::io::Result<()> {
    use std::io::{Read, Write};

    std::fs::create_dir_all(dest)?;
    let file = std::fs::File::open(jar_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if entry.is_dir() {
            continue;
        }
        let Some(entry_path) = entry.enclosed_name() else {
            continue;
        };
        let entry_path = entry_path.to_path_buf();
        let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Extract Java, Clojure, and Scala source files.
        // Clojure Maven artifacts ship .clj sources in standard jars; -sources.jar
        // variants contain .java stubs for Java interop classes.
        // Scala Maven artifacts (-sources.jar) contain .scala source files alongside
        // any .java interop shims — we need both to index external Scala libraries.
        // JVM-language sources jars ship a mix of files. We extract every
        // language extension we have a parser for; the walker downstream
        // dispatches to the right extractor by suffix.
        if !name.ends_with(".java")
            && !name.ends_with(".clj")
            && !name.ends_with(".cljc")
            && !name.ends_with(".scala")
            && !name.ends_with(".kt")
            && !name.ends_with(".kts")
            && !name.ends_with(".groovy")
        {
            continue;
        }
        let out_path = dest.join(&entry_path);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out_file = std::fs::File::create(&out_path)?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf)?;
        out_file.write_all(&buf)?;
    }
    Ok(())
}

/// Find the first subdirectory under `dir`.
pub(crate) fn find_first_subdir(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?
        .flatten()
        .find(|e| e.path().is_dir())
        .map(|e| e.path())
}

/// Maximum directory traversal depth for all ecosystem walkers.
pub(crate) const MAX_WALK_DEPTH: u32 = 20;

#[cfg(test)]
#[path = "externals_tests.rs"]
mod tests;
