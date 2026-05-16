// =============================================================================
// ecosystem/maven/discovery.rs
// =============================================================================


use std::path::{Path, PathBuf};
use std::sync::Arc;

use rayon::prelude::*;
use tracing::{debug, warn};
use tree_sitter::{Node, Parser};

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{
    collect_pom_files_bounded, coursier_cache_root, extract_java_sources_jar, gradle_caches_root,
    is_cache_stale, maven_local_repo, resolve_coursier_sources_jar, resolve_coursier_submodule_jars,
    resolve_gradle_sources_jar, resolve_maven_artifact_dir, ExternalDepRoot, ExternalSourceLocator,
    MAX_WALK_DEPTH,
};
use crate::ecosystem::manifest::maven::{parse_pom_xml_coords, MavenCoord};
use crate::ecosystem::manifest::{
    clojure as clojure_manifest,
    gradle as gradle_manifest,
    sbt as sbt_manifest,
};
use crate::walker::WalkedFile;
use super::ID;
use super::reachability::{collect_jvm_user_imports, walk_maven_narrowed};

// ---------------------------------------------------------------------------
// Discovery: walk every JVM manifest, collect coords, resolve against ~/.m2
// ---------------------------------------------------------------------------

pub(crate) fn discover_maven_roots(project_root: &Path) -> Vec<ExternalDepRoot> {
    let m2 = maven_local_repo();
    let gradle_cache = gradle_caches_root();
    let coursier_cache = coursier_cache_root();
    if m2.is_none() && gradle_cache.is_none() && coursier_cache.is_none() {
        debug!("No Maven, Gradle, or Coursier cache discovered; skipping JVM externals");
        return Vec::new();
    }

    // Pick a shared bearwisdom-sources-cache anchor. Prefer ~/.m2/.. so
    // existing extracted caches are reused; fall back to ~/.gradle/.. for
    // pure-Gradle dev machines, then Coursier for SBT-only setups.
    let cache_anchor = m2
        .as_deref()
        .or(gradle_cache.as_deref())
        .or(coursier_cache.as_deref())
        .map(|p| p.parent().unwrap_or(p).to_path_buf())
        .expect("at least one cache root");
    let cache_base = cache_anchor.join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    // R3 narrowing: collect every JVM `import` statement the user writes.
    // Each artifact's ExternalDepRoot carries the full set; walk_maven_narrowed
    // filters to only the package dirs actually referenced, collapsing the
    // cost of extracting spring-core or scala-library (~1000s of classes each)
    // to just the handful of packages the project consumes.
    let user_imports: Vec<String> = collect_jvm_user_imports(project_root)
        .into_iter()
        .collect();

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut missing_sources_jars: Vec<String> = Vec::new();

    // --- pom.xml coords (Java + Scala + Kotlin when Maven-built) --------
    let mut pom_paths: Vec<PathBuf> = Vec::new();
    collect_pom_files_bounded(project_root, &mut pom_paths, 0);
    let mut pom_coords = Vec::new();
    for pom in &pom_paths {
        let Ok(content) = std::fs::read_to_string(pom) else { continue };
        pom_coords.extend(parse_pom_xml_coords(&content));
    }
    debug!("Maven: {} pom.xml coords across {} files", pom_coords.len(), pom_paths.len());
    for coord in &pom_coords {
        resolve_and_push_jvm(
            m2.as_deref(),
            gradle_cache.as_deref(),
            coursier_cache.as_deref(),
            &cache_base,
            coord,
            &user_imports,
            &mut roots,
            &mut seen,
            &mut missing_sources_jars,
        );
    }

    // --- Gradle build.gradle[.kts] + version-catalog coords -------------
    let gradle_coords = collect_gradle_coords(project_root);
    debug!("Gradle: {} coords from build.gradle + libs.versions.toml", gradle_coords.len());
    for coord in &gradle_coords {
        resolve_and_push_jvm(
            m2.as_deref(),
            gradle_cache.as_deref(),
            coursier_cache.as_deref(),
            &cache_base,
            coord,
            &user_imports,
            &mut roots,
            &mut seen,
            &mut missing_sources_jars,
        );
    }

    // --- Scala sbt coords ----------------------------------------------
    // sbt's `%%` operator appends the active Scala version suffix to the
    // artifact name (`cats-core` → `cats-core_3` for Scala 3,
    // `cats-core_2.13` for 2.13). Without evaluating the build we can't
    // know which one the user is on, so probe each suffix in turn against
    // every cache. First hit wins.
    //
    // The triples carry a manifest-pinned version when it can be resolved
    // (via `val NAME = "X.Y.Z"` bindings across every sbt manifest) — that
    // pinned version gets passed to MavenCoord so the resolver targets the
    // right version directly. When unresolved, fall back to the lex-scan
    // over cached versions inside `resolve_maven_artifact_dir`.
    let scala_suffixes = ["_3", "_2.13", "_2.12", ""];
    for (group, artifact_base, version) in collect_sbt_coord_triples(project_root) {
        let mut hit = false;
        for suffix in &scala_suffixes {
            let artifact_id = format!("{artifact_base}{suffix}");
            let coord = MavenCoord {
                group_id: group.clone(),
                artifact_id,
                version: version.clone(),
            };
            let before = roots.len();
            resolve_and_push_jvm(
                m2.as_deref(),
                gradle_cache.as_deref(),
                coursier_cache.as_deref(),
                &cache_base,
                &coord,
                &user_imports,
                &mut roots,
                &mut seen,
                &mut missing_sources_jars,
            );
            if roots.len() > before {
                hit = true;
                break;
            }
        }
        if !hit {
            debug!(group = %group, artifact = %artifact_base, "sbt: sources jar not found in any cache");
        }
    }

    // --- Clojure deps.edn + project.clj coords --------------------------
    for dep in collect_clojure_deps(project_root) {
        let parts: Vec<&str> = dep.splitn(2, '/').collect();
        let (group_id, artifact_id) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            (dep.as_str(), dep.as_str())
        };
        let coord = MavenCoord {
            group_id: group_id.to_string(),
            artifact_id: artifact_id.to_string(),
            version: None,
        };
        resolve_and_push_jvm(
            m2.as_deref(),
            gradle_cache.as_deref(),
            coursier_cache.as_deref(),
            &cache_base,
            &coord,
            &user_imports,
            &mut roots,
            &mut seen,
            &mut missing_sources_jars,
        );
    }

    debug!("JVM ecosystem: {} total external dep roots", roots.len());
    if !missing_sources_jars.is_empty() {
        // Sources jars are an opt-in artifact (`mvn dependency:sources` /
        // Gradle `idea`/`eclipse` task / sbt `updateClassifiers`). Without
        // them the binary jar exists but BearWisdom can't index its symbols.
        // Surface a single summary so users know how to recover resolution
        // for declared deps that didn't resolve.
        let preview: Vec<&str> = missing_sources_jars
            .iter()
            .take(5)
            .map(|s| s.as_str())
            .collect();
        let suffix = if missing_sources_jars.len() > preview.len() {
            format!(", … and {} more", missing_sources_jars.len() - preview.len())
        } else {
            String::new()
        };
        warn!(
            "Maven: {} declared JVM deps have no -sources.jar in the local caches \
             ({}{}). Run `mvn dependency:sources` (or sbt `updateClassifiers` / \
             Gradle `:dependencies --refresh-dependencies` with sources)\
             to populate them.",
            missing_sources_jars.len(),
            preview.join(", "),
            suffix,
        );
    }
    roots
}

/// Parse every build.gradle[.kts] in the project and resolve catalog
/// references against any `gradle/*.versions.toml` files. Returns full
/// `MavenCoord`s — coords that omit `version` (rare in Gradle) get the
/// version-dir scan fallback the same way pom coords do.
fn collect_gradle_coords(project_root: &Path) -> Vec<MavenCoord> {
    let mut catalogs: std::collections::HashMap<String, gradle_manifest::GradleCatalog> =
        std::collections::HashMap::new();
    for (name, path) in gradle_manifest::collect_version_catalogs(project_root) {
        if let Ok(content) = std::fs::read_to_string(&path) {
            catalogs.insert(name, gradle_manifest::parse_version_catalog(&content));
        }
    }

    let mut out = Vec::new();
    for build_file in gradle_manifest::collect_gradle_build_files(project_root) {
        let Ok(content) = std::fs::read_to_string(&build_file) else { continue };
        out.extend(gradle_manifest::parse_gradle_coords(&content, &catalogs));
    }
    out
}

/// Try ~/.m2 first (preferred — single jar per artifact dir), fall back to
/// ~/.gradle/caches (hash-bucketed layout), then to Coursier's cache (used
/// by SBT's `updateClassifiers`). The first cache that yields a
/// `<artifact>-<version>-sources.jar` wins; all missing → silent skip.
fn resolve_and_push_jvm(
    m2: Option<&Path>,
    gradle_cache: Option<&Path>,
    coursier_cache: Option<&Path>,
    cache_base: &Path,
    coord: &MavenCoord,
    user_imports: &[String],
    roots: &mut Vec<ExternalDepRoot>,
    seen: &mut std::collections::HashSet<PathBuf>,
    missing_sources_jars: &mut Vec<String>,
) {
    // First pass: try the manifest-pinned version (when set) across every
    // cache. If nothing has the exact pinned version, fall back to a
    // version-blind probe so the walker still picks SOMETHING — a
    // close-enough version is more useful than no externals at all,
    // especially when the project's compile-classpath resolves to a
    // version that isn't pinned in the manifest verbatim.
    let resolved = try_resolve_in_caches(m2, gradle_cache, coursier_cache, coord)
        .or_else(|| {
            if coord.version.is_some() {
                let unpinned = MavenCoord {
                    group_id: coord.group_id.clone(),
                    artifact_id: coord.artifact_id.clone(),
                    version: None,
                };
                try_resolve_in_caches(m2, gradle_cache, coursier_cache, &unpinned)
            } else {
                None
            }
        });

    let Some((version, sources_jar)) = resolved else {
        debug!(
            "JVM sources jar missing for {}:{} (checked Maven + Gradle + Coursier) — skipping",
            coord.group_id, coord.artifact_id
        );
        missing_sources_jars.push(format!("{}:{}", coord.group_id, coord.artifact_id));
        return;
    };

    let cache_dir = cache_base
        .join(coord.group_id.replace('.', "_"))
        .join(&coord.artifact_id)
        .join(&version);
    if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
        if let Err(e) = extract_java_sources_jar(&sources_jar, &cache_dir) {
            debug!("Failed to extract {}: {e}", sources_jar.display());
            return;
        }
    }

    // Aggregator check: some Scala libraries (e.g. ScalaTest, Cats) publish
    // a top-level artifact whose -sources.jar is an empty shell containing
    // only META-INF. The real sources live in constituent sub-module jars
    // in the same Coursier group directory (e.g. scalatest-core_2.13,
    // scalatest-shouldmatchers_2.13). When the extracted cache dir is empty,
    // probe Coursier for those sub-module jars.
    let cache_is_empty = std::fs::read_dir(&cache_dir)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if cache_is_empty {
        if let Some(coursier) = coursier_cache {
            // Derive the prefix by stripping the Scala binary-version suffix.
            let artifact_base = strip_scala_version_suffix(&coord.artifact_id);
            let sub_jars = resolve_coursier_submodule_jars(
                coursier,
                &coord.group_id,
                artifact_base,
                Some(&version),
            );
            for (sub_artifact, sub_version, sub_jar) in sub_jars {
                let sub_cache = cache_base
                    .join(coord.group_id.replace('.', "_"))
                    .join(&sub_artifact)
                    .join(&sub_version);
                if !sub_cache.exists() || is_cache_stale(&sub_jar, &sub_cache) {
                    if let Err(e) = extract_java_sources_jar(&sub_jar, &sub_cache) {
                        debug!("Failed to extract sub-module {sub_artifact}: {e}");
                        continue;
                    }
                }
                if !seen.insert(sub_cache.clone()) { continue; }
                roots.push(ExternalDepRoot {
                    module_path: format!("{}:{}", coord.group_id, sub_artifact),
                    version: sub_version,
                    root: sub_cache,
                    ecosystem: ID.as_str(),
                    package_id: None,
                    requested_imports: user_imports.to_vec(),
                });
            }
        }
        // The aggregator itself has no source files — don't push it as a root.
        return;
    }

    if !seen.insert(cache_dir.clone()) { return }
    roots.push(ExternalDepRoot {
        module_path: format!("{}:{}", coord.group_id, coord.artifact_id),
        version,
        root: cache_dir,
        ecosystem: ID.as_str(),
        package_id: None,
        requested_imports: user_imports.to_vec(),
    });
}

/// Strip a Scala binary-version suffix from an artifact name.
/// `scalatest_2.13` → `scalatest`, `cats-core_3` → `cats-core`.
fn strip_scala_version_suffix(artifact: &str) -> &str {
    for suffix in &["_2.13", "_2.12", "_2.11", "_3"] {
        if let Some(base) = artifact.strip_suffix(suffix) {
            return base;
        }
    }
    artifact
}

/// Probe each cache (Maven local, Gradle, Coursier) for a sources jar
/// matching `coord`. First hit wins. The `coord.version` field is honored:
/// when set, every cache is asked for that specific version; when None,
/// each cache falls back to its own version-blind newest-wins logic.
fn try_resolve_in_caches(
    m2: Option<&Path>,
    gradle_cache: Option<&Path>,
    coursier_cache: Option<&Path>,
    coord: &MavenCoord,
) -> Option<(String, PathBuf)> {
    if let Some(repo) = m2 {
        if let Some((version, artifact_dir)) = resolve_maven_artifact_dir(repo, coord) {
            let sources_jar = artifact_dir.join(format!(
                "{}-{}-sources.jar",
                coord.artifact_id, version
            ));
            if sources_jar.is_file() {
                return Some((version, sources_jar));
            }
        }
    }
    if let Some(cache) = gradle_cache {
        if let Some(hit) = resolve_gradle_sources_jar(cache, coord) {
            return Some(hit);
        }
    }
    if let Some(cache) = coursier_cache {
        if let Some(hit) = resolve_coursier_sources_jar(cache, coord) {
            return Some(hit);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Scala sbt coord resolution (with _2.13, _3 suffix probing)
// ---------------------------------------------------------------------------

fn collect_sbt_artifacts(project_root: &Path) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    let build_sbt = project_root.join("build.sbt");
    if let Ok(content) = std::fs::read_to_string(&build_sbt) {
        all.extend(sbt_manifest::parse_sbt_deps(&content));
    }
    let deps_scala = project_root.join("project").join("Dependencies.scala");
    if let Ok(content) = std::fs::read_to_string(&deps_scala) {
        for dep in sbt_manifest::parse_sbt_deps(&content) {
            if !all.contains(&dep) {
                all.push(dep);
            }
        }
    }
    all
}

/// Walk every sbt manifest in the project (root build.sbt + project/*.sbt
/// + project/Dependencies.scala) and return `(group, artifact)` pairs.
/// Sub-projects in monorepos often have their own build.sbt under
/// `<project>/<module>/build.sbt`; collect those too.
fn collect_sbt_coord_pairs(project_root: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    let mut sbt_files = Vec::new();
    collect_sbt_files(project_root, &mut sbt_files, 0);

    for path in sbt_files {
        let Ok(content) = std::fs::read_to_string(&path) else { continue };
        for pair in sbt_manifest::parse_sbt_coord_pairs(&content) {
            if seen.insert(pair.clone()) {
                out.push(pair);
            }
        }
    }
    out
}

/// Variant that also carries the manifest-pinned version when it can be
/// resolved through `val NAME = "X.Y.Z"` bindings collected across every
/// sbt manifest in the project. Returns `(group, artifact, Option<version>)`
/// so the resolver can target the right version directory directly instead
/// of falling back to a (broken) lex-sort over whatever's in the cache.
fn collect_sbt_coord_triples(project_root: &Path) -> Vec<(String, String, Option<String>)> {
    let mut sbt_files = Vec::new();
    collect_sbt_files(project_root, &mut sbt_files, 0);

    // First pass: union all `val NAME = "VERSION"` bindings across every
    // manifest. sbt convention scatters them between root build.sbt and
    // project/Dependencies.scala — collect both before resolving deps.
    let mut vars: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for path in &sbt_files {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        for (k, v) in sbt_manifest::parse_sbt_version_vars(&content) {
            vars.insert(k, v);
        }
    }

    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    for path in &sbt_files {
        let Ok(content) = std::fs::read_to_string(path) else { continue };
        for triple in sbt_manifest::parse_sbt_coord_triples(&content, &vars) {
            let key = (triple.0.clone(), triple.1.clone());
            // Last write wins for version: a later manifest mention with a
            // resolved version overrides an earlier coord-only mention.
            if let Some(existing_idx) = out.iter().position(|t| t.0 == triple.0 && t.1 == triple.1) {
                if out[existing_idx].2.is_none() && triple.2.is_some() {
                    out[existing_idx].2 = triple.2;
                }
            } else if seen.insert(key) {
                out.push(triple);
            }
        }
    }
    out
}

fn collect_sbt_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    ".git" | "target" | "build" | "node_modules"
                        | ".gradle" | ".idea" | ".bsp"
                ) || name.starts_with('.') {
                    continue;
                }
            }
            collect_sbt_files(&path, out, depth + 1);
        } else if path.is_file() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with(".sbt") || name == "Dependencies.scala" {
                    out.push(path);
                }
            }
        }
    }
}

fn find_scala_source_jar(
    repo: &Path,
    artifact: &str,
    cache_base: &Path,
) -> Option<(String, String, String, PathBuf)> {
    let suffixes = ["_3", "_2.13", "_2.12", ""];
    for suffix in &suffixes {
        let full_artifact = format!("{artifact}{suffix}");
        if let Some((group, version, cache_dir)) =
            scan_maven_for_scala_artifact(repo, &full_artifact, cache_base)
        {
            return Some((group, full_artifact, version, cache_dir));
        }
    }
    None
}

fn scan_maven_for_scala_artifact(
    repo: &Path,
    artifact: &str,
    cache_base: &Path,
) -> Option<(String, String, PathBuf)> {
    fn scan_dir(
        dir: &Path,
        artifact: &str,
        cache_base: &Path,
        group_parts: &mut Vec<String>,
        depth: u32,
    ) -> Option<(String, String, PathBuf)> {
        if depth > 10 { return None }
        let Ok(entries) = std::fs::read_dir(dir) else { return None };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') { continue }
            let path = entry.path();
            if !path.is_dir() { continue }

            if name_str.as_ref() == artifact {
                if let Ok(versions) = std::fs::read_dir(&path) {
                    let mut version_dirs: Vec<PathBuf> = versions
                        .flatten()
                        .filter(|e| e.path().is_dir())
                        .map(|e| e.path())
                        .collect();
                    version_dirs.sort();
                    for vdir in version_dirs.iter().rev() {
                        let ver = vdir.file_name()?.to_str()?;
                        let sources_jar = vdir.join(format!("{artifact}-{ver}-sources.jar"));
                        if sources_jar.is_file() {
                            let group = group_parts.join(".");
                            let cache_dir = cache_base
                                .join(group.replace('.', "_"))
                                .join(artifact)
                                .join(ver);
                            if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
                                if extract_java_sources_jar(&sources_jar, &cache_dir).is_err() {
                                    continue;
                                }
                            }
                            return Some((group, ver.to_string(), cache_dir));
                        }
                    }
                }
            } else {
                group_parts.push(name_str.to_string());
                if let result @ Some(_) = scan_dir(&path, artifact, cache_base, group_parts, depth + 1) {
                    return result;
                }
                group_parts.pop();
            }
        }
        None
    }

    let mut group_parts = Vec::new();
    scan_dir(repo, artifact, cache_base, &mut group_parts, 0)
}

// ---------------------------------------------------------------------------
// Clojure deps collection (deps.edn + project.clj, recursive)
// ---------------------------------------------------------------------------

fn collect_clojure_deps(project_root: &Path) -> Vec<String> {
    let mut all: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    collect_clojure_deps_recursive(project_root, &mut all, &mut seen, 0);
    all
}

fn collect_clojure_deps_recursive(
    dir: &Path,
    all: &mut Vec<String>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 3;
    if !seen.insert(dir.to_path_buf()) { return }

    if let Ok(content) = std::fs::read_to_string(dir.join("project.clj")) {
        for dep in clojure_manifest::parse_project_clj_deps(&content) {
            if !all.contains(&dep) { all.push(dep); }
        }
    }
    if let Ok(content) = std::fs::read_to_string(dir.join("deps.edn")) {
        for dep in clojure_manifest::parse_deps_edn_deps(&content) {
            if !all.contains(&dep) { all.push(dep); }
        }
    }
    if depth >= MAX_DEPTH { return }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if matches!(
            name,
            ".git" | "target" | "out" | "node_modules" | ".clj-kondo"
                | ".lsp" | ".cpcache" | "resources" | "doc" | "docs"
        ) { continue }
        collect_clojure_deps_recursive(&path, all, seen, depth + 1);
    }
}

