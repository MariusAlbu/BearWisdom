// indexer/manifest/npm.rs — package.json reader

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct NpmManifest;

impl ManifestReader for NpmManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Npm
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let mut package_json_files = Vec::new();
        collect_package_json(project_root, &mut package_json_files, 0);

        if package_json_files.is_empty() {
            return None;
        }

        let mut data = ManifestData::default();

        for path in &package_json_files {
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for pkg in parse_package_json_deps(&content) {
                // For scoped packages like @tanstack/react-query, also add the scope.
                if pkg.starts_with('@') {
                    if let Some(scope) = pkg.split('/').next() {
                        data.dependencies.insert(scope.to_string());
                    }
                }
                data.dependencies.insert(pkg);
            }
        }

        // Add Node.js built-in module names (always external for TS/JS projects).
        for builtin in NODE_BUILTINS {
            data.dependencies.insert(builtin.to_string());
        }
        // Also add the node: protocol prefix as a sentinel.
        data.dependencies.insert("node".to_string());

        Some(data)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn collect_package_json(dir: &Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
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

/// Extract dependency package names from a package.json file's
/// `dependencies` and `devDependencies` objects.
///
/// Uses `serde_json` for parsing since it's already a workspace dependency.
pub fn parse_package_json_deps(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut packages = Vec::new();
    for key in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(serde_json::Value::Object(deps)) = obj.get(*key) {
            for pkg_name in deps.keys() {
                if !pkg_name.is_empty() {
                    packages.push(pkg_name.clone());
                }
            }
        }
    }
    packages
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
