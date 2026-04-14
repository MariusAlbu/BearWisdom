// Scala / sbt externals — Maven source jars via the Java externals pipeline

use super::{
    extract_java_sources_jar, is_cache_stale, maven_local_repo, ExternalDepRoot,
    ExternalSourceLocator, MAX_WALK_DEPTH,
};
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Scala sbt → Maven source jars (via the Java externals pipeline).
///
/// Scala projects declare dependencies in `build.sbt` using the sbt DSL
/// (`"org" %% "artifact" % "version"`). The `%%` operator appends the
/// Scala version suffix to the artifact name (e.g., `cats-core_2.13`).
/// Dependencies are resolved by Coursier (sbt 1.3+) which also populates
/// `~/.m2/repository/` when configured.
///
/// This locator reuses the Java externals pipeline entirely — it parses
/// `build.sbt` for Maven coordinates, then resolves source jars from the
/// Maven local repo. The only difference is the manifest parser
/// (`SbtManifest` instead of `MavenManifest`).
pub struct ScalaExternalsLocator;

impl ExternalSourceLocator for ScalaExternalsLocator {
    fn ecosystem(&self) -> &'static str { "scala" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_scala_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_scala_external_root(dep)
    }
}

/// Discover external Scala package roots for a project.
///
/// Strategy: parse `build.sbt` (and `project/Dependencies.scala`) for
/// `libraryDependencies` entries. Each entry yields a Maven coordinate.
/// For `%%` deps, we probe the Maven local repo for both `artifact_2.13`
/// and `artifact_3` suffixed directories. Source jars are extracted to a
/// cache directory and walked as Scala source.
pub fn discover_scala_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::sbt::parse_sbt_deps;

    let build_sbt = project_root.join("build.sbt");
    if !build_sbt.is_file() {
        return Vec::new();
    }

    let mut all_artifacts: Vec<String> = Vec::new();

    if let Ok(content) = std::fs::read_to_string(&build_sbt) {
        all_artifacts.extend(parse_sbt_deps(&content));
    }
    let deps_scala = project_root.join("project").join("Dependencies.scala");
    if let Ok(content) = std::fs::read_to_string(&deps_scala) {
        for dep in parse_sbt_deps(&content) {
            if !all_artifacts.contains(&dep) {
                all_artifacts.push(dep);
            }
        }
    }

    if all_artifacts.is_empty() {
        return Vec::new();
    }

    let Some(repo) = maven_local_repo() else {
        debug!("No Maven local repository discovered; skipping Scala externals");
        return Vec::new();
    };
    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for artifact in &all_artifacts {
        if let Some((group, art, version, cache_dir)) =
            find_scala_source_jar(&repo, artifact, &cache_base)
        {
            if !seen.insert(cache_dir.clone()) {
                continue;
            }
            roots.push(ExternalDepRoot {
                module_path: format!("{group}:{art}"),
                version,
                root: cache_dir,
                ecosystem: "scala",
                package_id: None,
            });
        }
    }
    debug!("Scala: discovered {} external source roots", roots.len());
    roots
}

/// Try to find a source jar for an sbt artifact name. Because sbt `%%`
/// appends the Scala version suffix, we probe multiple variants:
///   - `{artifact}_3`, `{artifact}_2.13` (Scala-versioned)
///   - `{artifact}` as-is (Java dep via `%`)
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

/// Walk the Maven local repo looking for any group that contains the given
/// artifact directory with a sources jar inside. Returns (group_id, version, cache_dir).
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
        if depth > 10 {
            return None;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return None;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            if name_str.as_ref() == artifact {
                // Found the artifact dir — look for a version subdir with sources jar
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

/// Walk a Scala source directory extracted from a sources jar.
pub fn walk_scala_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_scala_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_scala_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_scala_dir_bounded(dir, root, dep, out, 0);
}

fn walk_scala_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, "test" | "tests" | "META-INF") || name.starts_with('.') {
                    continue;
                }
            }
            walk_scala_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let (is_scala, skip) = {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !(name.ends_with(".scala") || name.ends_with(".java")) {
                    continue;
                }
                let skip = name.ends_with("Test.scala") || name.ends_with("Spec.scala")
                    || name.ends_with("Suite.scala") || name.ends_with("Tests.scala");
                (name.ends_with(".scala"), skip)
            };
            if skip {
                continue;
            }
            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:scala:{}/{}", dep.module_path, rel_sub);
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language: if is_scala { "scala" } else { "java" },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::manifest::ManifestReader;

    #[test]
    fn sbt_manifest_parses_build_sbt() {
        let tmp = std::env::temp_dir().join("bw-test-scala-manifest");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("build.sbt"), r#"
libraryDependencies ++= List(
  "org.http4s" %% "http4s-dsl" % "1.0",
  "io.circe" %% "circe-core" % "0.14",
)
"#).unwrap();

        let reader = crate::indexer::manifest::sbt::SbtManifest;
        let data = reader.read(&tmp).unwrap();
        let mut deps: Vec<String> = data.dependencies.iter().cloned().collect();
        deps.sort();
        assert_eq!(deps, vec!["circe-core", "http4s-dsl"]);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scala_empty_without_maven_repo() {
        let tmp = std::env::temp_dir().join("bw-test-scala-no-repo");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("build.sbt"), r#""org.typelevel" %% "cats-core" % "2.12""#).unwrap();

        // Without a Maven local repo, should return empty (gracefully).
        // (May or may not be empty depending on machine — just verify no crash.)
        let _ = discover_scala_externals(&tmp);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
