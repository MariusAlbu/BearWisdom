// indexer/manifest/clojure.rs — project.clj / deps.edn reader

use std::path::Path;
use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct ClojureManifest;

impl ManifestReader for ClojureManifest {
    fn kind(&self) -> ManifestKind { ManifestKind::Clojure }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut data = ManifestData::default();
        let mut found = false;

        let project_clj = project_root.join("project.clj");
        if project_clj.is_file() {
            if let Ok(content) = std::fs::read_to_string(&project_clj) {
                found = true;
                for name in parse_project_clj_deps(&content) {
                    data.dependencies.insert(name);
                }
            }
        }

        let deps_edn = project_root.join("deps.edn");
        if deps_edn.is_file() {
            if let Ok(content) = std::fs::read_to_string(&deps_edn) {
                found = true;
                for name in parse_deps_edn_deps(&content) {
                    data.dependencies.insert(name);
                }
            }
        }

        if found { Some(data) } else { None }
    }

    /// Monorepo-aware walk: discovers sub-project `project.clj` / `deps.edn`
    /// files up to 3 directory levels deep.
    ///
    /// Leiningen multi-module projects (like the ring library) declare sub-modules
    /// via `:sub [...]` in the root `project.clj`, but each sub-module carries its
    /// own dependency list in its own `project.clj`. This override surfaces each
    /// sub-module as a separate `ReaderEntry` so `is_manifest_dependency` can find
    /// `ring/ring-codec` when resolving symbols in `ring-core/src/`.
    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut out = Vec::new();
        collect_clojure_manifests(project_root, project_root, &mut out, 0);
        out
    }
}

/// Recursively walk `dir` (bounded depth) looking for `project.clj` / `deps.edn`.
/// Each manifest found becomes a separate `ReaderEntry` keyed to its directory.
/// Skips well-known non-source directories to avoid false positives.
fn collect_clojure_manifests(
    project_root: &Path,
    dir: &Path,
    out: &mut Vec<ReaderEntry>,
    depth: usize,
) {
    const MAX_DEPTH: usize = 3;

    // Try to read a manifest at this directory level.
    let mut data = ManifestData::default();
    let mut found = false;
    let mut manifest_path = dir.to_path_buf();

    let project_clj = dir.join("project.clj");
    if project_clj.is_file() {
        if let Ok(content) = std::fs::read_to_string(&project_clj) {
            found = true;
            manifest_path = project_clj;
            for name in parse_project_clj_deps(&content) {
                data.dependencies.insert(name);
            }
        }
    }
    let deps_edn = dir.join("deps.edn");
    if deps_edn.is_file() {
        if let Ok(content) = std::fs::read_to_string(&deps_edn) {
            if !found {
                manifest_path = deps_edn;
            }
            found = true;
            for name in parse_deps_edn_deps(&content) {
                data.dependencies.insert(name);
            }
        }
    }

    if found {
        // Extract a package name from the first `defproject` or `deps.edn` root key.
        let rel = dir.strip_prefix(project_root).unwrap_or(dir);
        let name = rel
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| {
                project_root
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "root".to_string());
        out.push(ReaderEntry {
            package_dir: dir.to_path_buf(),
            manifest_path,
            data,
            name: Some(name),
        });
    }

    if depth >= MAX_DEPTH {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Skip directories that never contain sub-project manifests.
        if matches!(
            name,
            ".git"
                | "target"
                | "out"
                | "node_modules"
                | ".clj-kondo"
                | ".lsp"
                | ".cpcache"
                | "resources"
                | "doc"
                | "docs"
        ) {
            continue;
        }
        collect_clojure_manifests(project_root, &path, out, depth + 1);
    }
}

/// Parse dependency names from project.clj `:dependencies` vector.
/// Format: `:dependencies [[ring/ring-core "1.15.3"] [org.clojure/data.json "2.4.0"]]`
pub fn parse_project_clj_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find(":dependencies") else { return deps; };
    let rest = &content[start..];
    let Some(bracket) = rest.find('[') else { return deps; };
    let rest = &rest[bracket + 1..];

    // Find matching close bracket, tracking depth
    let mut depth = 1u32;
    let mut end = 0;
    for (i, ch) in rest.char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 { end = i; break; }
            }
            _ => {}
        }
    }
    let block = &rest[..end];

    // Each dep is [artifact "version" ...] — extract artifact from inner vectors
    let mut inner_depth = 0u32;
    let mut dep_start = 0usize;
    for (i, ch) in block.char_indices() {
        match ch {
            '[' => {
                inner_depth += 1;
                if inner_depth == 1 { dep_start = i + 1; }
            }
            ']' => {
                if inner_depth == 1 {
                    let dep_text = block[dep_start..i].trim();
                    let name = dep_text.split_whitespace().next().unwrap_or("");
                    if !name.is_empty() && name != "org.clojure/clojure" {
                        deps.push(name.to_string());
                    }
                }
                inner_depth = inner_depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    deps
}

/// Parse dependency names from deps.edn `:deps` map.
/// Format: `:deps {org.clojure/data.json {:mvn/version "2.4.0"} ...}`
pub fn parse_deps_edn_deps(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    let Some(start) = content.find(":deps") else { return deps; };
    let rest = &content[start + ":deps".len()..];
    let rest = rest.trim();
    if !rest.starts_with('{') { return deps; }
    let rest = &rest[1..];

    let mut depth = 1u32;
    let mut end = 0;
    for (i, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 { end = i; break; }
            }
            _ => {}
        }
    }
    let block = &rest[..end];

    // Keys alternate with values. Keys are symbols, values are maps.
    // Split on top-level `{...}` blocks: text before each `{` is a key.
    let mut map_depth = 0u32;
    let mut segment_start = 0usize;
    for (i, ch) in block.char_indices() {
        match ch {
            '{' => {
                if map_depth == 0 {
                    let key = block[segment_start..i].trim();
                    if !key.is_empty() && !key.starts_with(':') && key != "org.clojure/clojure" {
                        deps.push(key.to_string());
                    }
                }
                map_depth += 1;
            }
            '}' => {
                map_depth = map_depth.saturating_sub(1);
                if map_depth == 0 {
                    segment_start = i + 1;
                }
            }
            _ => {}
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lein_deps() {
        let content = r#"(defproject ring "1.15.3"
  :dependencies [[ring/ring-core "1.15.3"]
                 [ring/ring-devel "1.15.3"]
                 [ring/ring-jetty-adapter "1.15.3"]])"#;
        let deps = parse_project_clj_deps(content);
        assert_eq!(deps, vec!["ring/ring-core", "ring/ring-devel", "ring/ring-jetty-adapter"]);
    }

    #[test]
    fn parse_deps_edn() {
        let content = r#"{:deps {org.clojure/data.json {:mvn/version "2.4.0"}
               ring/ring-core {:mvn/version "1.9.0"}}}"#;
        let deps = parse_deps_edn_deps(content);
        assert!(deps.contains(&"org.clojure/data.json".to_string()));
        assert!(deps.contains(&"ring/ring-core".to_string()));
    }
}
