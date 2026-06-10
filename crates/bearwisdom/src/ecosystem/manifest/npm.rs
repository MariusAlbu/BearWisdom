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
                data.path_aliases = parse_tsconfig_paths_with_extends(&tsconfig_path);
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

/// Parse `compilerOptions.paths` from a tsconfig.json file.
///
/// Returns `(alias_prefix, target_prefix)` tuples with trailing `*` stripped.
/// Exact-match entries (no wildcard) come through with empty-string sentinels
/// reserved via a trailing `=` — here we only surface prefix-mapped entries
/// because those are what the resolver rewrites. Exact alias matches are a
/// rare special case and not worth the extra bookkeeping today.
///
/// Strips `//` line comments and `/* */` block comments before JSON parsing
/// so valid JSONC tsconfigs don't fail. Does not follow `extends`.
pub fn parse_tsconfig_paths(content: &str) -> Vec<(String, String)> {
    let stripped = strip_json_comments(content);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return Vec::new();
    };
    let Some(paths) = value
        .get("compilerOptions")
        .and_then(|co| co.get("paths"))
        .and_then(|p| p.as_object())
    else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (key, targets) in paths {
        // Only wildcard-mapped aliases: `"@/*": ["src/*"]`. Strip the
        // trailing `*` on both sides to get bare prefix strings.
        let Some(alias_prefix) = key.strip_suffix('*') else {
            continue;
        };
        let Some(arr) = targets.as_array() else {
            continue;
        };
        let Some(first) = arr.first().and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(target_prefix) = first.strip_suffix('*') else {
            continue;
        };
        if alias_prefix.is_empty() {
            continue;
        }
        out.push((alias_prefix.to_string(), target_prefix.to_string()));
    }
    out
}

/// Like `parse_tsconfig_paths` but follows the `extends` chain, so a package
/// that declares its `paths` in a shared base config (common in monorepos and
/// `@tsconfig/*` presets) still contributes aliases. Resolves both relative
/// (`./base.json`, `../tsconfig.base.json`) and package
/// (`@org/cfg/web.json`, `@tsconfig/node18/tsconfig.json`) `extends` targets.
/// Child entries win over inherited ones on key conflict. Bounded depth with a
/// visited-set cycle guard.
pub fn parse_tsconfig_paths_with_extends(tsconfig_path: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_tsconfig_paths(
        tsconfig_path,
        &|p| std::fs::read_to_string(p).ok(),
        &mut out,
        &mut seen,
        0,
    );
    out
}

const MAX_TSCONFIG_EXTENDS_DEPTH: usize = 8;

/// Walk a tsconfig and its `extends` ancestors, accumulating path aliases.
/// `read` returns a file's content or `None` when it doesn't exist — folding
/// existence and content into one closure keeps the extends-resolution logic
/// testable without touching the filesystem.
fn collect_tsconfig_paths(
    path: &Path,
    read: &dyn Fn(&Path) -> Option<String>,
    out: &mut Vec<(String, String)>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    if depth >= MAX_TSCONFIG_EXTENDS_DEPTH || !seen.insert(path.to_path_buf()) {
        return;
    }
    let Some(content) = read(path) else { return };
    // First-writer-wins: the current (more derived) config's aliases are pushed
    // before its ancestors', so a child key shadows the parent's.
    for entry in parse_tsconfig_paths(&content) {
        if !out.iter().any(|(k, _)| *k == entry.0) {
            out.push(entry);
        }
    }
    let base_dir = path.parent().unwrap_or_else(|| Path::new(""));
    for target in tsconfig_extends_targets(&content) {
        for candidate in extends_candidate_paths(&target, base_dir) {
            if read(&candidate).is_some() {
                collect_tsconfig_paths(&candidate, read, out, seen, depth + 1);
                break;
            }
        }
    }
}

/// Extract the `extends` field as a list of targets. TS 5.0+ allows an array;
/// older configs use a single string.
fn tsconfig_extends_targets(content: &str) -> Vec<String> {
    let stripped = strip_json_comments(content);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&stripped) else {
        return Vec::new();
    };
    match value.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(serde_json::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Candidate on-disk locations for an `extends` target, in priority order.
/// Relative targets resolve against `base_dir` (a missing `.json` is appended);
/// package targets resolve under `node_modules`, walking up from `base_dir`,
/// trying `<pkg>.json` then `<pkg>/tsconfig.json`.
fn extends_candidate_paths(target: &str, base_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if target.starts_with('.') {
        let rel = target.trim_start_matches("./");
        let joined = base_dir.join(rel);
        if joined.extension().is_some() {
            out.push(joined);
        } else {
            out.push(base_dir.join(format!("{rel}.json")));
        }
        return out;
    }
    let mut dir = Some(base_dir);
    while let Some(d) = dir {
        let nm = d.join("node_modules").join(target);
        if target.ends_with(".json") {
            out.push(nm);
        } else {
            out.push(nm.with_extension("json"));
            out.push(nm.join("tsconfig.json"));
        }
        dir = d.parent();
    }
    out
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
/// Same JSONC tolerance as `parse_tsconfig_paths` — strips `//` and
/// `/* */` comments before parsing. Does not follow `extends`.
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
fn strip_json_comments(src: &str) -> String {
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
