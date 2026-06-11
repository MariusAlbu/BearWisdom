// =============================================================================
// ecosystem/maven_classes.rs — Java bytecode walker for declared jars
//
// Maven/Gradle projects declare deps via pom.xml/build.gradle; the build
// downloads each dep's `.jar` (bytecode) into the local cache. The
// optional `-sources.jar` (containing `.java` sources) is opt-in — most
// projects don't fetch them. The `MavenEcosystem` source path walks
// sources-jars when present but is blind to the bytecode-only jars.
// This walker fills that gap: for each dependency coordinate the project
// declares, when no `-sources.jar` exists in the caches, the matching
// bytecode `.jar` is cracked and its public/protected types + members are
// emitted as ParsedFile entries.
//
// **Activation:** transitive on Maven — if Maven didn't already fire,
// neither did source-jar discovery, so there's nothing for us to fill
// in.
//
// **Discovery is coordinate-driven:** jars are located by probing the
// `.m2` / Gradle / Coursier caches for the exact group:artifact:version
// the project declares, never by crawling the machine-wide cache. A
// coordinate whose `-sources.jar` is already cached is skipped here — the
// source path indexes it instead.
//
// **Performance:** the bytecode parse runs only on the declared
// dependency set. Signed/agent jars are handled by `jar_walker`; anything
// > 32 MB (rare fat shaded artefacts that bog down the parser) is skipped.
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{
    coursier_cache_root, gradle_caches_root, maven_local_repo, resolve_coursier_bytecode_jar,
    resolve_gradle_bytecode_jar, resolve_maven_artifact_dir, ExternalDepRoot,
    ExternalSourceLocator,
};
use crate::ecosystem::jar_walker;
use crate::ecosystem::manifest::maven::MavenCoord;
use crate::ecosystem::maven::{
    collect_declared_jvm_coords, collect_workspace_artifact_ids, jvm_sources_jar_available,
    ID as MAVEN_ID,
};
use crate::types::ParsedFile;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("maven-classes");
const ECOSYSTEM_TAG: &str = "maven-classes";
const LANGUAGES: &[&str] = &["java"];
/// Cap per-jar parse cost — anything bigger is almost certainly a fat
/// shaded artefact that'd dominate index time without proportional
/// resolution value.
const MAX_JAR_BYTES: u64 = 32 * 1024 * 1024;
/// Safety bound on jars cracked per project. The coordinate-driven probe
/// already bounds the set to declared deps; this only guards a manifest
/// that declares a pathological number of coordinates.
const MAX_JARS_PER_PROJECT: usize = 300;

pub struct MavenClassesEcosystem;

impl Ecosystem for MavenClassesEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::TransitiveOn(MAVEN_ID)
    }

    fn locate_roots(&self, _ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // No on-disk root layout — everything happens via parse_metadata_only.
        Vec::new()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }
}

impl ExternalSourceLocator for MavenClassesEcosystem {
    fn ecosystem(&self) -> &'static str {
        ECOSYSTEM_TAG
    }
    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        Vec::new()
    }
    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<ParsedFile>> {
        let jars = discover_jars(project_root);
        if jars.is_empty() {
            return None;
        }
        let mut out: Vec<ParsedFile> = Vec::new();
        for (i, jar) in jars.into_iter().enumerate() {
            if i >= MAX_JARS_PER_PROJECT {
                break;
            }
            if let Ok(meta) = fs::metadata(&jar) {
                if meta.len() > MAX_JAR_BYTES {
                    continue;
                }
            }
            out.extend(jar_walker::walk_jar(&jar));
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

/// Locate the bytecode `.jar` for each dependency the project declares.
///
/// Two sources, both project-scoped:
///   * **Project-local** `lib/`, `libs/`, `vendor/`, `deps/` directories —
///     jars vendored in the repo itself.
///   * **Declared coordinates** — for every `group:artifact:version` in the
///     project's pom/Gradle manifests, probe the `.m2`, Gradle, and Coursier
///     caches for that exact coordinate's bytecode jar. A coordinate whose
///     `-sources.jar` is already cached is skipped: the source path indexes
///     it, and re-indexing the bytecode would duplicate the symbols.
///
/// The `-sources`/`-javadoc`/`-tests` classifier jars are never returned as
/// the bytecode jar.
fn discover_jars(project_root: &Path) -> Vec<PathBuf> {
    discover_jars_in_caches(
        project_root,
        maven_local_repo().as_deref(),
        gradle_caches_root().as_deref(),
        coursier_cache_root().as_deref(),
    )
}

/// Core of `discover_jars` with the cache roots passed explicitly, so the
/// coordinate-driven probe can be exercised against fixture caches without
/// touching process-global cache-discovery env vars.
fn discover_jars_in_caches(
    project_root: &Path,
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
) -> Vec<PathBuf> {
    let mut jars = Vec::new();

    // Project-local `lib/`, `libs/`, `vendor/`, `deps/` — common in older
    // projects that check jars into the repo. This stays a directory scan
    // because it's scoped to the project tree, not the machine.
    for sub in &["lib", "libs", "vendor", "deps"] {
        let dir = project_root.join(sub);
        if dir.is_dir() {
            collect_local_jars(&dir, &mut jars, 0);
        }
    }

    // The workspace's own module ids — a coordinate naming one resolves to
    // project build output, never a cached jar. These are indexed from
    // project source, so exclude them from the bytecode probe.
    let own_modules = collect_workspace_artifact_ids(project_root);

    for coord in collect_declared_jvm_coords(project_root) {
        if own_modules.contains(&coord.artifact_id) {
            continue;
        }
        // The source-jar path already covers any coordinate with a cached
        // `-sources.jar`; cracking its bytecode here would double-index.
        if jvm_sources_jar_available(m2, gradle, coursier, &coord) {
            continue;
        }
        if let Some(jar) = resolve_jvm_bytecode_jar(m2, gradle, coursier, &coord) {
            jars.push(jar);
        }
    }

    jars
}

/// Probe the `.m2`, Gradle, and Coursier caches (in that precedence order)
/// for `coord`'s bytecode jar. First hit wins; `None` when the coordinate
/// isn't cached in any layout. Honors `coord.version`, falling back to the
/// largest cached version when absent or dynamic.
///
/// When the manifest-pinned version isn't cached, a second pass retries with
/// the version dropped so a close-enough cached version still resolves. This
/// mirrors the sources-path fallback in `resolve_and_push_jvm`: a project
/// often pins a version the local cache doesn't hold verbatim (the compile
/// classpath resolved a different one), and a near version's bytecode is far
/// more useful than no externals at all.
fn resolve_jvm_bytecode_jar(
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
    coord: &MavenCoord,
) -> Option<PathBuf> {
    if let Some(jar) = resolve_jvm_bytecode_jar_pinned(m2, gradle, coursier, coord) {
        return Some(jar);
    }
    if coord.version.is_some() {
        let unpinned = MavenCoord {
            group_id: coord.group_id.clone(),
            artifact_id: coord.artifact_id.clone(),
            version: None,
        };
        return resolve_jvm_bytecode_jar_pinned(m2, gradle, coursier, &unpinned);
    }
    None
}

/// Single-pass probe across the three caches for `coord` exactly as given —
/// the pinned version when set, each cache's newest-wins fallback when None.
fn resolve_jvm_bytecode_jar_pinned(
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
    coord: &MavenCoord,
) -> Option<PathBuf> {
    if let Some(repo) = m2 {
        if let Some((version, artifact_dir)) = resolve_maven_artifact_dir(repo, coord) {
            let jar = artifact_dir.join(format!("{}-{}.jar", coord.artifact_id, version));
            if jar.is_file() {
                return Some(jar);
            }
        }
    }
    if let Some(cache) = gradle {
        if let Some(jar) = resolve_gradle_bytecode_jar(cache, coord) {
            return Some(jar);
        }
    }
    if let Some(cache) = coursier {
        if let Some(jar) = resolve_coursier_bytecode_jar(cache, coord) {
            return Some(jar);
        }
    }
    None
}

/// Recursively collect `.jar` files under a project-local directory,
/// excluding the `-sources`/`-javadoc`/`-tests` classifier variants.
fn collect_local_jars(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    if depth > 10 || out.len() > MAX_JARS_PER_PROJECT {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() > MAX_JARS_PER_PROJECT {
            return;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_local_jars(&path, out, depth + 1);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".jar") {
                continue;
            }
            if name.ends_with("-sources.jar")
                || name.ends_with("-javadoc.jar")
                || name.ends_with("-tests.jar")
            {
                continue;
            }
            out.push(path);
        }
    }
}

#[cfg(test)]
pub(super) fn _test_discover_jars_in_caches(
    project_root: &Path,
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
) -> Vec<PathBuf> {
    discover_jars_in_caches(project_root, m2, gradle, coursier)
}

#[cfg(test)]
pub(super) fn _test_resolve_jvm_bytecode_jar(
    m2: Option<&Path>,
    gradle: Option<&Path>,
    coursier: Option<&Path>,
    coord: &MavenCoord,
) -> Option<PathBuf> {
    resolve_jvm_bytecode_jar(m2, gradle, coursier, coord)
}

#[cfg(test)]
#[path = "maven_classes_tests.rs"]
mod tests;
