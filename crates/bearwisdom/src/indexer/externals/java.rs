// Java — Maven local repository + sources jar walker

use super::{
    collect_pom_files_bounded, extract_java_sources_jar, is_cache_stale, maven_local_repo,
    resolve_maven_artifact_dir, ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH,
};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Maven local repository → `discover_java_externals` + `walk_java_external_root`.
/// Source jars are extracted on demand by the discovery pass; this locator
/// returns the extracted directory roots.
pub struct JavaExternalsLocator;

impl ExternalSourceLocator for JavaExternalsLocator {
    fn ecosystem(&self) -> &'static str { "java" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_java_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_java_external_root(dep)
    }
}

/// Discover all external Java dependency roots for a project.
///
/// Strategy:
/// 1. Parse every `pom.xml` under the project root via the existing Maven
///    manifest reader — returns full `MavenCoord` triples (groupId,
///    artifactId, version).
/// 2. Locate the Maven local repository in this order:
///    - `BEARWISDOM_JAVA_MAVEN_REPO` env override
///    - `$HOME/.m2/repository` (or `%USERPROFILE%\.m2\repository` on Windows)
/// 3. For each coord, compute the artifact directory
///    `{repo}/{groupId.replace('.', '/')}/{artifactId}/{version}` and look
///    for the sources jar `{artifactId}-{version}-sources.jar` inside it.
///    When the pom didn't specify a version, scan the artifact directory
///    and pick the lexicographically latest subdirectory as the version.
/// 4. Extract the jar's `.java` entries to a persistent cache directory
///    `{repo}/../bearwisdom-sources-cache/{coord_slug}/` so re-indexing
///    stays fast. Returns one `ExternalDepRoot` per dep pointing at the
///    cache directory.
///
/// Test jars (`-test-sources.jar`) and classifier-suffixed variants are
/// skipped intentionally — they aren't part of the public API surface and
/// would inflate the symbol table with test scaffolding.
pub fn discover_java_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::maven::parse_pom_xml_coords;

    // Collect every pom.xml under the project. Reusing the MavenManifest
    // walker would only surface groupIds, so we re-walk here for the full
    // coordinates.
    let mut pom_paths: Vec<PathBuf> = Vec::new();
    collect_pom_files_bounded(project_root, &mut pom_paths, 0);
    if pom_paths.is_empty() {
        return Vec::new();
    }

    let mut coords = Vec::new();
    for pom in &pom_paths {
        let Ok(content) = std::fs::read_to_string(pom) else {
            continue;
        };
        coords.extend(parse_pom_xml_coords(&content));
    }
    if coords.is_empty() {
        return Vec::new();
    }

    let Some(repo) = maven_local_repo() else {
        debug!("No Maven local repository discovered; skipping Java externals");
        return Vec::new();
    };
    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    debug!(
        "Probing Maven local repo {} for {} declared coords",
        repo.display(),
        coords.len()
    );

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for coord in coords {
        let Some((version, artifact_dir)) = resolve_maven_artifact_dir(&repo, &coord) else {
            continue;
        };
        let sources_jar = artifact_dir.join(format!(
            "{}-{}-sources.jar",
            coord.artifact_id, version
        ));
        if !sources_jar.is_file() {
            debug!(
                "Maven sources jar missing for {}:{}:{} — skipping",
                coord.group_id, coord.artifact_id, version
            );
            continue;
        }

        let cache_dir = cache_base
            .join(coord.group_id.replace('.', "_"))
            .join(&coord.artifact_id)
            .join(&version);
        if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
            if let Err(e) = extract_java_sources_jar(&sources_jar, &cache_dir) {
                debug!(
                    "Failed to extract {}: {e}",
                    sources_jar.display()
                );
                continue;
            }
        }

        if !seen.insert(cache_dir.clone()) {
            continue;
        }
        roots.push(ExternalDepRoot {
            module_path: format!("{}:{}", coord.group_id, coord.artifact_id),
            version,
            root: cache_dir,
            ecosystem: "java",
            package_id: None,
        });
    }
    roots
}

/// Walk one Java external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Only `.java` files.
/// - Skip `package-info.java` (package-level annotations only) and
///   `module-info.java` (JPMS module descriptor, not public API).
/// - Skip `tests/`, `test/`, `*Test.java`, `*Tests.java`.
///
/// Virtual relative_path is `ext:java:{groupId:artifactId}/{sub_path}`.
pub fn walk_java_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_java_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_java_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_java_dir_bounded(dir, root, dep, out, 0);
}

fn walk_java_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") {
                    continue;
                }
            }
            walk_java_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.ends_with(".java") {
                continue;
            }
            if name == "package-info.java" || name == "module-info.java" {
                continue;
            }
            if name.ends_with("Test.java") || name.ends_with("Tests.java") {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:java:{}/{}", dep.module_path, rel_sub);

            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: "java",
            });
        }
    }
}
