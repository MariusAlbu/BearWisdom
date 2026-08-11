// =============================================================================
// ecosystem/coursier_cache — locate JVM dependency jars in the Coursier cache
//
// The Coursier layout nests group segments as directories under per-protocol
// roots; a process-wide index of artifact directories is built once per cache
// root and consulted for sources / bytecode / submodule jar resolution, with
// newest-version fallback when the pinned version is absent.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::externals::{pick_newest_version_from_dir, strip_scala_suffix};

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
            v.push(
                PathBuf::from(local)
                    .join("Coursier")
                    .join("Cache")
                    .join("v1"),
            );
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
        v.push(
            PathBuf::from(home)
                .join(".cache")
                .join("coursier")
                .join("v1"),
        );
        v
    };
    candidates.into_iter().find(|p| p.is_dir())
}

/// Maximum directory depth walked while indexing the Coursier cache. The
/// scheme/host/repo-base wrappers plus the deepest Maven group path
/// (`org/springframework/boot/spring-boot-starter`) and a version dir stay
/// well under this; the bound only caps a pathological cache.
const COURSIER_INDEX_DEPTH: u32 = 12;

/// Process-wide memo of each Coursier cache's artifact-directory index, keyed
/// by cache root. Built once per cache root by a single full walk, then shared
/// across every coordinate lookup in the index — the discovery loop resolves
/// hundreds of coordinates against the same cache, so a per-coordinate tree
/// scan is quadratic. The index maps `artifact_id` to every on-disk artifact
/// directory of that name; a coordinate lookup filters those by the trailing
/// `<group-as-path>/<artifact>` to pin the exact group.
static COURSIER_INDEX: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<CoursierIndex>>>,
> = std::sync::OnceLock::new();

/// One Coursier cache's artifact directories, grouped by artifact id. An
/// artifact directory is the parent of `<version>/` dirs — i.e. the path
/// `<repo-base>/<group-as-path>/<artifact>`. Keyed by `<artifact>` for a
/// cheap first-level filter; group disambiguation is a path-suffix check.
struct CoursierIndex {
    by_artifact: std::collections::HashMap<String, Vec<PathBuf>>,
}

impl CoursierIndex {
    /// Every artifact directory matching `<group-as-path>/<artifact>` — the
    /// deterministic Maven layout tail, independent of the repo-base prefix
    /// (Central's `maven2`, a Nexus `content/repositories/…`, etc.).
    fn artifact_dirs(&self, group_id: &str, artifact_id: &str) -> Vec<PathBuf> {
        let Some(candidates) = self.by_artifact.get(artifact_id) else {
            return Vec::new();
        };
        let suffix = group_artifact_suffix(group_id, artifact_id);
        candidates
            .iter()
            .filter(|dir| path_ends_with_components(dir, &suffix))
            .cloned()
            .collect()
    }

    /// Every group directory (`<repo-base>/<group-as-path>`) holding an
    /// artifact whose name begins with `artifact_prefix` (Scala suffix
    /// stripped). Used by aggregator sub-module expansion, which enumerates
    /// sibling artifacts under one group dir. Deduplicated.
    fn group_dirs(&self, group_id: &str) -> Vec<PathBuf> {
        let suffix = group_components(group_id);
        let mut out: Vec<PathBuf> = Vec::new();
        for dirs in self.by_artifact.values() {
            for dir in dirs {
                let Some(group_dir) = dir.parent() else {
                    continue;
                };
                if path_ends_with_components(group_dir, &suffix) && !out.iter().any(|p| p == group_dir)
                {
                    out.push(group_dir.to_path_buf());
                }
            }
        }
        out
    }
}

/// `group_id` dotted-to-path-components plus the artifact id, for suffix
/// matching against an absolute artifact dir.
fn group_artifact_suffix(group_id: &str, artifact_id: &str) -> Vec<String> {
    let mut parts: Vec<String> = group_id.split('.').map(|s| s.to_string()).collect();
    parts.push(artifact_id.to_string());
    parts
}

fn group_components(group_id: &str) -> Vec<String> {
    group_id.split('.').map(|s| s.to_string()).collect()
}

/// True when `path`'s trailing components equal `components` (last-to-first).
fn path_ends_with_components(path: &Path, components: &[String]) -> bool {
    let mut actual = path.components().rev();
    for want in components.iter().rev() {
        match actual.next() {
            Some(std::path::Component::Normal(os)) if os.to_str() == Some(want.as_str()) => {}
            _ => return false,
        }
    }
    true
}

/// Get (building once) the artifact-directory index for `cache_root`.
fn coursier_index(cache_root: &Path) -> Arc<CoursierIndex> {
    let key = std::fs::canonicalize(cache_root).unwrap_or_else(|_| cache_root.to_path_buf());
    let map = COURSIER_INDEX.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = map.lock().expect("coursier index map poisoned");
    if let Some(idx) = guard.get(&key) {
        return Arc::clone(idx);
    }
    let idx = Arc::new(build_coursier_index(cache_root));
    guard.insert(key, Arc::clone(&idx));
    idx
}

/// Walk `cache_root` once, registering every artifact directory (the parent of
/// a `<version>/` dir that directly contains a `.jar`) under its leaf name.
fn build_coursier_index(cache_root: &Path) -> CoursierIndex {
    fn walk(
        dir: &Path,
        depth: u32,
        by_artifact: &mut std::collections::HashMap<String, Vec<PathBuf>>,
    ) {
        if depth >= COURSIER_INDEX_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut has_jar = false;
        let mut subdirs: Vec<PathBuf> = Vec::new();
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                subdirs.push(entry.path());
            } else if ft.is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".jar") {
                        has_jar = true;
                    }
                }
            }
        }
        // A dir holding a `.jar` is a version dir; its parent is the artifact
        // dir. Register and stop — the version/jar layer has no sub-artifacts.
        if has_jar {
            if let Some(artifact_dir) = dir.parent() {
                if let Some(name) = artifact_dir.file_name().and_then(|n| n.to_str()) {
                    by_artifact
                        .entry(name.to_string())
                        .or_default()
                        .push(artifact_dir.to_path_buf());
                }
            }
            return;
        }
        for sub in subdirs {
            walk(&sub, depth + 1, by_artifact);
        }
    }

    let mut by_artifact: std::collections::HashMap<String, Vec<PathBuf>> =
        std::collections::HashMap::new();
    walk(cache_root, 0, &mut by_artifact);
    for dirs in by_artifact.values_mut() {
        dirs.sort();
        dirs.dedup();
    }
    CoursierIndex { by_artifact }
}

/// Every on-disk `<repo-base>/<group-as-path>/<artifact>` directory for a
/// coordinate, via the cache's memoized artifact-directory index.
fn coursier_artifact_dirs(cache_root: &Path, group_id: &str, artifact_id: &str) -> Vec<PathBuf> {
    coursier_index(cache_root).artifact_dirs(group_id, artifact_id)
}

/// Pick the artifact's `(version, version_dir)` for `coord` in a Coursier
/// artifact directory — the pinned version when set, else the newest cached.
fn coursier_version_dir(
    artifact_dir: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<(String, PathBuf)> {
    let version = match &coord.version {
        Some(v) => v.clone(),
        None => pick_newest_version_from_dir(artifact_dir)?,
    };
    let version_dir = artifact_dir.join(&version);
    if version_dir.is_dir() {
        Some((version, version_dir))
    } else {
        None
    }
}

/// Resolve a `MavenCoord` against the Coursier cache. Probes the deterministic
/// `<group-path>/<artifact>/<version>/<artifact>-<version>-sources.jar` under
/// each repo root (Maven Central, Sonatype, custom) via direct path join; the
/// first hit wins.
pub(crate) fn resolve_coursier_sources_jar(
    cache_root: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<(String, PathBuf)> {
    for artifact_dir in coursier_artifact_dirs(cache_root, &coord.group_id, &coord.artifact_id) {
        let Some((version, version_dir)) = coursier_version_dir(&artifact_dir, coord) else {
            continue;
        };
        let jar = version_dir.join(format!("{}-{}-sources.jar", coord.artifact_id, version));
        if jar.is_file() {
            return Some((version, jar));
        }
    }
    None
}

/// Resolve a `MavenCoord` to its bytecode `.jar` in the Coursier cache.
/// Direct-joins the deterministic `<group-path>/<artifact>/<version>/` under
/// each repo root like `resolve_coursier_sources_jar`, matching
/// `<artifact>-<version>.jar` while excluding the `-sources`/`-javadoc`/`-tests`
/// classifier variants. When `coord.version` is None, picks the largest
/// cached version.
pub(crate) fn resolve_coursier_bytecode_jar(
    cache_root: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<PathBuf> {
    for artifact_dir in coursier_artifact_dirs(cache_root, &coord.group_id, &coord.artifact_id) {
        let Some((version, version_dir)) = coursier_version_dir(&artifact_dir, coord) else {
            continue;
        };
        let jar = version_dir.join(format!("{}-{}.jar", coord.artifact_id, version));
        if jar.is_file() {
            return Some(jar);
        }
    }
    None
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
    let mut out = Vec::new();
    // A group can be cached under more than one repo root (Central + a
    // snapshot repo). Probe the deterministic group path under each and
    // gather sub-modules from all of them.
    for group_dir in coursier_group_dirs(cache_root, group_id) {
        let Ok(entries) = std::fs::read_dir(&group_dir) else {
            continue;
        };
        collect_coursier_submodules(entries, artifact_prefix, preferred_version, &mut out);
    }
    out
}

/// Every on-disk `<repo-base>/<group-as-path>` directory for `group_id` — the
/// parent of the artifact dirs, used by sub-module aggregator expansion — via
/// the cache's memoized artifact-directory index.
fn coursier_group_dirs(cache_root: &Path, group_id: &str) -> Vec<PathBuf> {
    coursier_index(cache_root).group_dirs(group_id)
}

/// Collect sub-module sources jars whose base artifact name (Scala suffix
/// stripped) begins with `artifact_prefix` but is not the aggregator itself.
fn collect_coursier_submodules(
    entries: std::fs::ReadDir,
    artifact_prefix: &str,
    preferred_version: Option<&str>,
    out: &mut Vec<(String, String, PathBuf)>,
) {
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(artifact_dir_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        // Strip the Scala version suffix to get the base artifact name, then
        // check that it starts with our prefix. `scalatest-core_2.13` →
        // base = `scalatest-core`; `scalatest_2.13` itself is the aggregator
        // we already processed — skip it.
        let base = strip_scala_suffix(artifact_dir_name);
        if !base.starts_with(artifact_prefix) {
            continue;
        }
        // Skip the aggregator itself (exact match after suffix strip).
        if base == artifact_prefix {
            continue;
        }

        // Pick version: preferred first, then newest available.
        let version = if let Some(v) = preferred_version {
            let vdir = path.join(v);
            if vdir.is_dir() {
                v.to_string()
            } else {
                let Some(newest) = pick_newest_version_from_dir(&path) else {
                    continue;
                };
                newest
            }
        } else {
            let Some(newest) = pick_newest_version_from_dir(&path) else {
                continue;
            };
            newest
        };

        let sources_jar = path
            .join(&version)
            .join(format!("{artifact_dir_name}-{version}-sources.jar"));
        if sources_jar.is_file() {
            out.push((artifact_dir_name.to_string(), version, sources_jar));
        }
    }
}
