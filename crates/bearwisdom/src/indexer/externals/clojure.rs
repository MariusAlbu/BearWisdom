// Clojure / Leiningen + deps.edn externals — Maven source jars via the Java pipeline

use super::{
    extract_java_sources_jar, is_cache_stale, maven_local_repo, resolve_maven_artifact_dir,
    ExternalDepRoot, ExternalSourceLocator,
};
use crate::walker::WalkedFile;
use std::path::Path;
use tracing::debug;

/// Clojure Leiningen/deps.edn → Maven source jars (via the Java externals pipeline).
///
/// Clojure deps are Maven artifacts with group/artifact coordinates. Both
/// `project.clj` (Leiningen) and `deps.edn` (tools.deps) are parsed.
/// Source jars are resolved from `~/.m2/repository/` and extracted to a cache.
/// Walk delegates to `walk_java_external_root` since the extracted sources
/// are standard Java/Clojure `.java`/`.clj` files.
pub struct ClojureExternalsLocator;

impl ExternalSourceLocator for ClojureExternalsLocator {
    fn ecosystem(&self) -> &'static str { "clojure" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_clojure_externals(project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_clojure_external_root(dep)
    }
}

pub fn discover_clojure_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    use crate::indexer::manifest::clojure::{parse_deps_edn_deps, parse_project_clj_deps};

    let mut all_deps: Vec<String> = Vec::new();

    let project_clj = project_root.join("project.clj");
    if let Ok(content) = std::fs::read_to_string(&project_clj) {
        all_deps.extend(parse_project_clj_deps(&content));
    }
    let deps_edn = project_root.join("deps.edn");
    if let Ok(content) = std::fs::read_to_string(&deps_edn) {
        for dep in parse_deps_edn_deps(&content) {
            if !all_deps.contains(&dep) { all_deps.push(dep); }
        }
    }
    if all_deps.is_empty() { return Vec::new(); }

    let Some(repo) = maven_local_repo() else { return Vec::new(); };
    let cache_base = repo.parent().unwrap_or(&repo).join("bearwisdom-sources-cache");
    let _ = std::fs::create_dir_all(&cache_base);

    let mut roots = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for dep in &all_deps {
        // Clojure deps are group/artifact format
        let parts: Vec<&str> = dep.splitn(2, '/').collect();
        let (group_id, artifact_id) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            (dep.as_str(), dep.as_str())
        };

        let coord = crate::indexer::manifest::maven::MavenCoord {
            group_id: group_id.to_string(),
            artifact_id: artifact_id.to_string(),
            version: None,
        };
        let Some((version, artifact_dir)) = resolve_maven_artifact_dir(&repo, &coord) else { continue; };
        let sources_jar = artifact_dir.join(format!("{artifact_id}-{version}-sources.jar"));
        if !sources_jar.is_file() { continue; }

        let cache_dir = cache_base
            .join(group_id.replace('.', "_"))
            .join(artifact_id)
            .join(&version);
        if !cache_dir.exists() || is_cache_stale(&sources_jar, &cache_dir) {
            if extract_java_sources_jar(&sources_jar, &cache_dir).is_err() { continue; }
        }
        if !seen.insert(cache_dir.clone()) { continue; }
        roots.push(ExternalDepRoot {
            module_path: dep.clone(),
            version,
            root: cache_dir,
            ecosystem: "clojure",
            package_id: None,
        });
    }
    debug!("Clojure: discovered {} external source roots", roots.len());
    roots
}

pub fn walk_clojure_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    // Delegate to the Java walker — extracted source jars have the same layout.
    super::java::walk_java_external_root(dep)
}
