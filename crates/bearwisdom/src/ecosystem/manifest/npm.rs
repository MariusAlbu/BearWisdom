// indexer/manifest/npm.rs — package.json reader

use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};

#[path = "tsconfig_paths.rs"]
pub mod tsconfig_paths;
pub use tsconfig_paths::{
    parse_tsconfig_aliases, parse_tsconfig_aliases_with_extends, parse_tsconfig_paths,
    parse_tsconfig_paths_with_extends, TsconfigAliases,
};

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
            data.dependencies
                .extend(e.data.dependencies.iter().cloned());
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
            let Ok(content) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };

            let mut data = ManifestData::default();
            data.package_entries = super::npm_entry::package_entries(&content);
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

            // tsconfig.json aliases live alongside package.json for TS
            // packages. Missing or unparseable tsconfig is non-fatal — the
            // resolver just won't rewrite aliases for this package.
            let tsconfig_path = package_dir.join("tsconfig.json");
            if let Ok(ts_content) = std::fs::read_to_string(&tsconfig_path) {
                // Follow `extends` so monorepo / preset base configs that hold
                // the `paths` map still contribute aliases.
                let aliases = parse_tsconfig_aliases_with_extends(&tsconfig_path);
                data.path_aliases = aliases.prefixes;
                data.exact_path_aliases = aliases.exact;
                data.tsconfig_types = parse_tsconfig_types(&ts_content);
            }

            // Vite / Vue CLI / webpack / Nuxt configs also carry `resolve.alias`
            // prefix mappings. Parse each probed filename and append anything
            // found so the resolver's existing alias rewrite treats them
            // identically to tsconfig paths. JS/TS ASTs are walked — static
            // values only; anything dynamic is dropped.
            const JS_CONFIG_FILES: &[&str] = &[
                "vite.config.ts",
                "vite.config.js",
                "vite.config.mjs",
                "vite.config.mts",
                "vue.config.js",
                "vue.config.ts",
                "webpack.config.js",
                "webpack.config.ts",
                "nuxt.config.ts",
                "nuxt.config.js",
                // SvelteKit declares aliases under `kit.alias` (e.g.
                // `$lib -> src/lib`). The generated `.svelte-kit/tsconfig.json`
                // carries the same `paths`, but the root tsconfig only reaches
                // them through `extends`, which `parse_tsconfig_paths` does not
                // follow — svelte.config is the authoritative source.
                "svelte.config.js",
                "svelte.config.ts",
                "svelte.config.mjs",
            ];
            let mut has_any_js_config = false;
            let mut declares_at_alias = false;
            for file_name in JS_CONFIG_FILES {
                let cfg_path = package_dir.join(file_name);
                let Ok(cfg_content) = std::fs::read_to_string(&cfg_path) else {
                    continue;
                };
                has_any_js_config = true;
                let extra = super::js_config_aliases::parse_js_config_aliases(&cfg_content);
                for entry in extra {
                    if entry.0 == "@/" {
                        declares_at_alias = true;
                    }
                    // Longest-match wins in the resolver, so duplicate keys
                    // across config files are harmless — we just push them.
                    data.path_aliases.push(entry);
                }
            }

            // Framework-convention default aliases. Some Vite plugins inject
            // path aliases at runtime rather than declaring them in the
            // user's config file. These conventions are widely enough used
            // that hard-coding them here (gated on the plugin being a
            // declared dependency) recovers thousands of unresolved refs in
            // Laravel / Nuxt / SvelteKit projects without needing a plugin
            // loader.
            if has_any_js_config && !declares_at_alias {
                // `laravel-vite-plugin` injects `@/` → `resources/js/` so
                // `import Foo from '@/Components/Foo.vue'` maps to
                // `resources/js/Components/Foo.vue`. Monica, Jetstream,
                // Breeze, and every Laravel + Inertia starter use this.
                let has_laravel_vite = data.dependencies.iter().any(|d| d == "laravel-vite-plugin");
                if has_laravel_vite && package_dir.join("resources").join("js").is_dir() {
                    data.path_aliases
                        .push(("@/".to_string(), "resources/js/".to_string()));
                }
            }

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

/// Parse `compilerOptions.types` from a tsconfig.json file.
///
/// `types` is a TypeScript-native mechanism declaring which packages
/// contribute AMBIENT globals — symbols available without an explicit
/// `import` statement. Common entries: `"vitest/globals"` (provides
/// `expect` / `describe` / `it`), `"node"` (provides `process` /
/// `Buffer`), `"@types/jest"`, `"@playwright/test"`, etc.
///
/// Returns the raw package name strings as listed. The resolver matches
/// them against external file paths (`node_modules/<name>/...`) to
/// classify which candidates supply ambient globals.
///
/// Strips `//` and `/* */` comments before parsing. Does not follow `extends`.
pub fn parse_tsconfig_types(content: &str) -> Vec<String> {
    let stripped = strip_json_comments(content);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return Vec::new();
    };
    let Some(arr) = value
        .get("compilerOptions")
        .and_then(|co| co.get("types"))
        .and_then(|t| t.as_array())
    else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Strip `//` line comments and `/* */` block comments, respecting strings
/// so we don't mangle URLs or paths that happen to contain `//`.
pub(super) fn strip_json_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b as char);
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        // `//` to end of line
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // `/* ... */`
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
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
                "node_modules"
                    | ".git"
                    | "target"
                    | "bin"
                    | "obj"
                    | ".next"
                    | "dist"
                    | "build"
                    | ".cache"
                    | "coverage"
                    | ".turbo"
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
#[path = "npm_tests.rs"]
mod tests;
