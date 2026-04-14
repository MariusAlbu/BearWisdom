// indexer/manifest/npm.rs — package.json reader

use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

pub struct NpmManifest;

impl ManifestReader for NpmManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Npm
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        // Node builtins are appended by read_all per-entry; ensure present on
        // the unioned result as well (idempotent).
        for builtin in NODE_BUILTINS {
            data.dependencies.insert(builtin.to_string());
        }
        data.dependencies.insert("node".to_string());
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut package_json_files = Vec::new();
        collect_package_json(project_root, &mut package_json_files, 0);

        let mut out = Vec::new();
        for manifest_path in package_json_files {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            let (name, deps) = parse_package_json(&content);
            for pkg in deps {
                if pkg.starts_with('@') {
                    if let Some(scope) = pkg.split('/').next() {
                        data.dependencies.insert(scope.to_string());
                    }
                }
                data.dependencies.insert(pkg);
            }
            // Node builtins are always externally resolvable from any TS/JS
            // package, regardless of what its own package.json declares.
            for builtin in NODE_BUILTINS {
                data.dependencies.insert(builtin.to_string());
            }
            data.dependencies.insert("node".to_string());

            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());

            out.push(ReaderEntry {
                package_dir,
                manifest_path,
                data,
                name,
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn collect_package_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                "node_modules" | ".git" | "target" | "bin" | "obj" | ".next"
                    | "dist" | "build" | ".cache" | "coverage" | ".turbo"
            ) {
                continue;
            }
            collect_package_json(&path, out, depth + 1);
        } else if entry.file_name() == "package.json" {
            out.push(path);
        }
    }
}

/// Parse a package.json file into (name, external-dep-names).
///
/// Reads `dependencies`, `devDependencies`, `peerDependencies` object keys and
/// the top-level `name` field. Returns `(None, [])` on parse failure.
///
/// **Workspace-protocol deps are excluded** — values starting with
/// `workspace:`, `file:`, `link:`, or `portal:` point at sibling packages
/// within the monorepo, not npm registry entries. Including them in the
/// external dep set causes the resolver to misclassify sibling imports
/// as external when they should resolve to cross-package edges.
fn parse_package_json(content: &str) -> (Option<String>, Vec<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return (None, Vec::new());
    };
    let Some(obj) = value.as_object() else {
        return (None, Vec::new());
    };

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut packages = Vec::new();
    for key in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for (pkg_name, version_value) in deps.iter() {
                if pkg_name.is_empty() {
                    continue;
                }
                let version = version_value.as_str().unwrap_or("");
                if is_workspace_protocol(version) {
                    continue;
                }
                packages.push(pkg_name.clone());
            }
        }
    }
    (name, packages)
}

/// True when the dep's version spec points at a sibling workspace package
/// rather than a registry entry. These must not pollute the external dep
/// set — they're handled by the workspace-package resolver instead.
///
/// Covers:
///   * `workspace:*`, `workspace:^`, `workspace:~`, `workspace:1.2.3` (pnpm/yarn)
///   * `file:../path/to/pkg` (npm file: protocol)
///   * `link:../path/to/pkg` (yarn link: protocol)
///   * `portal:../path/to/pkg` (yarn portal: protocol)
fn is_workspace_protocol(version: &str) -> bool {
    version.starts_with("workspace:")
        || version.starts_with("file:")
        || version.starts_with("link:")
        || version.starts_with("portal:")
}

/// Extract dependency package names only (for legacy callers).
///
/// Kept for backward compat with any external consumers of this helper.
pub fn parse_package_json_deps(content: &str) -> Vec<String> {
    parse_package_json(content).1
}

/// Node.js core module names. These are always external regardless of
/// whether they appear in package.json.
const NODE_BUILTINS: &[&str] = &[
    "assert",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "crypto",
    "dgram",
    "dns",
    "domain",
    "events",
    "fs",
    "http",
    "http2",
    "https",
    "inspector",
    "module",
    "net",
    "os",
    "path",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "repl",
    "stream",
    "string_decoder",
    "timers",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_registry_deps() {
        let json = r#"{"name": "@myorg/web", "dependencies": {"react": "^18.0.0", "axios": "1.2.3"}}"#;
        let (name, deps) = parse_package_json(json);
        assert_eq!(name.as_deref(), Some("@myorg/web"));
        assert!(deps.contains(&"react".to_string()));
        assert!(deps.contains(&"axios".to_string()));
    }

    #[test]
    fn workspace_protocol_deps_excluded() {
        let json = r#"{
            "name": "@myorg/web",
            "dependencies": {
                "react": "^18.0.0",
                "@myorg/utils": "workspace:*",
                "@myorg/ui": "workspace:^",
                "@myorg/local": "file:../local-pkg"
            }
        }"#;
        let (_, deps) = parse_package_json(json);
        assert!(deps.contains(&"react".to_string()), "registry dep kept");
        assert!(!deps.contains(&"@myorg/utils".to_string()), "workspace:* excluded");
        assert!(!deps.contains(&"@myorg/ui".to_string()), "workspace:^ excluded");
        assert!(!deps.contains(&"@myorg/local".to_string()), "file: excluded");
    }

    #[test]
    fn link_and_portal_protocols_excluded() {
        let json = r#"{
            "name": "app",
            "dependencies": {
                "pinned": "link:../pinned",
                "tunneled": "portal:../tunneled"
            }
        }"#;
        let (_, deps) = parse_package_json(json);
        assert!(deps.is_empty(), "link: and portal: both excluded");
    }

    #[test]
    fn mixed_workspace_and_registry_both_work() {
        let json = r#"{
            "name": "web",
            "dependencies": {"react": "^18"},
            "devDependencies": {
                "typescript": "^5",
                "@internal/test-utils": "workspace:*"
            }
        }"#;
        let (_, deps) = parse_package_json(json);
        assert!(deps.contains(&"react".to_string()));
        assert!(deps.contains(&"typescript".to_string()));
        assert!(!deps.contains(&"@internal/test-utils".to_string()));
    }

    #[test]
    fn workspace_protocol_helper_recognizes_variants() {
        assert!(is_workspace_protocol("workspace:*"));
        assert!(is_workspace_protocol("workspace:^"));
        assert!(is_workspace_protocol("workspace:~"));
        assert!(is_workspace_protocol("workspace:1.2.3"));
        assert!(is_workspace_protocol("file:../foo"));
        assert!(is_workspace_protocol("link:../foo"));
        assert!(is_workspace_protocol("portal:../foo"));

        assert!(!is_workspace_protocol("^1.0.0"));
        assert!(!is_workspace_protocol("1.2.3"));
        assert!(!is_workspace_protocol("git+https://github.com/x/y"));
        assert!(!is_workspace_protocol("npm:foo@1.0.0"));
    }
}
