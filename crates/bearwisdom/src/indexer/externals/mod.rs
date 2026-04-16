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
    coord: &crate::indexer::manifest::maven::MavenCoord,
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
        // Pick the lexicographically largest subdirectory — not perfect
        // semver ordering but good enough to find any cached version.
        let entries = std::fs::read_dir(&group_path).ok()?;
        let mut versions: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                if e.file_type().ok()?.is_dir() {
                    e.file_name().into_string().ok()
                } else {
                    None
                }
            })
            .collect();
        versions.sort();
        versions.into_iter().next_back()?
    };

    let artifact_dir = group_path.join(&version);
    if artifact_dir.is_dir() {
        Some((version, artifact_dir))
    } else {
        None
    }
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
        if !name.ends_with(".java")
            && !name.ends_with(".clj")
            && !name.ends_with(".cljc")
            && !name.ends_with(".scala")
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
