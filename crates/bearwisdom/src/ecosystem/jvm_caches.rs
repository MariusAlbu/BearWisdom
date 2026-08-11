// =============================================================================
// ecosystem/jvm_caches — locate JVM dependency sources in Maven and Gradle caches
//
// Pure filesystem probing: given a MavenCoord, find the artifact directory in
// the local Maven repository or the Gradle module cache and pick its sources /
// bytecode jar, falling back to the newest cached version when the pinned one
// is absent. No parsing, no extraction — jar handling stays with the callers.
// =============================================================================

use std::path::{Path, PathBuf};

use super::externals::{pick_newest_version, pick_newest_version_from_dir};

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
    // GRADLE_USER_HOME relocates the whole `~/.gradle` directory — Gradle
    // itself honors this to move the cache off the default user profile
    // (common in CI runners and shared-cache setups).
    if let Some(user_home) = std::env::var_os("GRADLE_USER_HOME") {
        let candidate = PathBuf::from(user_home)
            .join("caches")
            .join("modules-2")
            .join("files-2.1");
        if candidate.is_dir() {
            return Some(candidate);
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

/// Resolve a `MavenCoord` to its bytecode `.jar` in the Gradle cache.
/// Same `<group>/<artifact>/<version>/<hash>/` layout as
/// `resolve_gradle_sources_jar`, but matches `<artifact>-<version>.jar`
/// while excluding the `-sources`/`-javadoc`/`-tests` classifier variants.
/// When `coord.version` is None, picks the largest cached version.
pub(crate) fn resolve_gradle_bytecode_jar(
    cache_root: &Path,
    coord: &crate::ecosystem::manifest::maven::MavenCoord,
) -> Option<PathBuf> {
    let artifact_dir = cache_root.join(&coord.group_id).join(&coord.artifact_id);
    if !artifact_dir.is_dir() {
        return None;
    }

    let version = match &coord.version {
        Some(v) => v.clone(),
        None => pick_newest_version_from_dir(&artifact_dir)?,
    };

    let version_dir = artifact_dir.join(&version);
    if !version_dir.is_dir() {
        return None;
    }
    let target_name = format!("{}-{}.jar", coord.artifact_id, version);
    for entry in std::fs::read_dir(&version_dir).ok()?.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let candidate = p.join(&target_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
