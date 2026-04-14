// TypeScript / JavaScript node_modules discovery + walker

use super::{ts_package_from_virtual_path, ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH};
use crate::indexer::manifest::npm::NpmManifest;
use crate::indexer::manifest::ManifestReader;
use crate::walker::WalkedFile;
use std::path::{Path, PathBuf};
use tracing::debug;

/// node_modules + @types → `discover_ts_externals` + `walk_ts_external_root`.
/// Adds a post-process pass that rewrites TS declaration-file symbols to
/// `package.Symbol` qualified names so the Tier-1 resolver can match
/// `import { Button } from 'my-pkg'` → `my-pkg.Button`.
pub struct TypeScriptExternalsLocator;

impl ExternalSourceLocator for TypeScriptExternalsLocator {
    fn ecosystem(&self) -> &'static str { "typescript" }

    fn locate_roots(&self, project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_ts_externals(project_root)
    }

    /// M3: per-package discovery. Reads this package's own `package.json`
    /// and probes `{package}/node_modules` plus every ancestor node_modules
    /// walking up to `workspace_root` — covers npm/yarn-v1 hoisted layouts
    /// where shared deps live at the workspace root, not per-package.
    fn locate_roots_for_package(
        &self,
        workspace_root: &Path,
        package_abs_path: &Path,
        package_id: i64,
    ) -> Vec<ExternalDepRoot> {
        let mut roots = discover_ts_externals_scoped(workspace_root, package_abs_path);
        for r in &mut roots {
            r.package_id = Some(package_id);
        }
        roots
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_external_root(dep)
    }

    fn post_process_parsed(&self, parsed: &mut crate::types::ParsedFile) {
        if let Some(pkg) = ts_package_from_virtual_path(&parsed.path).map(str::to_string) {
            prefix_ts_external_symbols(parsed, &pkg);
        }
    }
}

/// Discover all external TypeScript/JavaScript dependency roots for a project.
///
/// Strategy:
/// 1. Read package.json(s) via the existing `NpmManifest` reader (already
///    walks subdirs and handles dependencies/devDependencies/peerDependencies
///    plus Node.js builtins).
/// 2. Locate node_modules via (in order) `BEARWISDOM_TS_NODE_MODULES` env
///    override, project-local `node_modules` at root and immediate subdirs
///    (monorepo layout).
/// 3. For each declared dep, resolve to `node_modules/{name}/` (scoped
///    packages like `@tanstack/react-query` map to `node_modules/@tanstack/react-query/`).
/// 4. Skip Node.js builtins — they don't have an on-disk source tree. The
///    NpmManifest reader adds them to the dep set but they don't exist
///    under node_modules.
/// 5. Skip packages whose directory is missing (not installed).
pub fn discover_ts_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let manifest = NpmManifest;
    let Some(data) = manifest.read(project_root) else {
        return Vec::new();
    };
    if data.dependencies.is_empty() {
        return Vec::new();
    }

    let node_modules_roots = find_node_modules(project_root);
    if node_modules_roots.is_empty() {
        debug!("No node_modules dirs discovered; skipping TS externals");
        return Vec::new();
    }
    debug!(
        "Probing {} node_modules root(s) for {} declared deps",
        node_modules_roots.len(),
        data.dependencies.len()
    );

    // Node builtins — these are declared as deps by NpmManifest but have no
    // on-disk source under node_modules. Skip them entirely; language
    // resolvers already route them via test_globals / runtime globals.
    let builtins: std::collections::HashSet<&'static str> = [
        "assert", "buffer", "child_process", "cluster", "console", "crypto",
        "dgram", "dns", "domain", "events", "fs", "http", "http2", "https",
        "inspector", "module", "net", "node", "os", "path", "perf_hooks",
        "process", "punycode", "querystring", "readline", "repl", "stream",
        "string_decoder", "timers", "tls", "trace_events", "tty", "url",
        "util", "v8", "vm", "wasi", "worker_threads", "zlib",
    ]
    .into_iter()
    .collect();

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep in &data.dependencies {
        if builtins.contains(dep.as_str()) {
            continue;
        }
        // Skip bare scope sentinels (`@tanstack`) — NpmManifest inserts these
        // alongside the real scoped package names; resolving them as a package
        // would incorrectly pull in the whole scope directory.
        if dep.starts_with('@') && !dep.contains('/') {
            continue;
        }
        // Skip `@types/X` entries (DefinitelyTyped type-only packages). The
        // real package `X` is typically also in deps, and the fallback probe
        // below will pull `node_modules/@types/X/` under the `X` module path
        // so its symbols get qualified as `X.Foo` — which is what the TS
        // resolver looks for on `import { Foo } from 'X'`. If we processed
        // `@types/X` as its own dep it would land as `@types/X.Foo` and
        // never match a user-code import.
        if dep.starts_with("@types/") {
            continue;
        }

        // Collect every on-disk root for this dep:
        //
        // 1. The primary `node_modules/{dep}/` directory (which may ship only
        //    `.js` if the package is untyped).
        // 2. The DefinitelyTyped sibling that carries `.d.ts` for untyped
        //    libraries:
        //    - Unscoped: `node_modules/@types/{dep}/`
        //    - Scoped:   `node_modules/@types/{scope}__{name}/`
        //      e.g. `@tanstack/react-query` → `@types/tanstack__react-query`.
        //      This is the escape scheme DefinitelyTyped uses on npm because
        //      `@` is not allowed inside an `@types/*` sub-path.
        //
        // Both roots share the same `module_path` so their symbols get the
        // same package prefix (`react.ReactNode`), and the Tier 1 TS
        // resolver's `{import_module}.{target}` lookup finds them equally.
        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            // Scoped package: `@foo/bar` → `node_modules/@foo/bar/`
            // Unscoped: `react` → `node_modules/react/`
            let primary = nm_root.join(dep);
            if primary.is_dir() {
                pkg_roots.push(primary);
            }
            // DefinitelyTyped fallback — unscoped and scoped both.
            if !dep.starts_with('@') {
                let types_dir = nm_root.join("@types").join(dep);
                if types_dir.is_dir() {
                    pkg_roots.push(types_dir);
                }
            } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                let types_dir = nm_root.join("@types").join(&escaped);
                if types_dir.is_dir() {
                    pkg_roots.push(types_dir);
                }
            }
        }

        for pkg_dir in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: "typescript",
                    package_id: None,
                });
            }
        }
    }
    roots
}

/// Convert a scoped npm package name into the DefinitelyTyped escape form.
///
/// DefinitelyTyped publishes types for scoped packages at
/// `@types/{scope}__{name}` because npm disallows nested `@` inside a scope
/// path. For example:
///
/// - `@tanstack/react-query` → `tanstack__react-query`
/// - `@radix-ui/react-dialog` → `radix-ui__react-dialog`
///
/// Returns `None` if the input isn't a scoped name (`@scope/name`) so callers
/// can skip the probe for unscoped deps, which take a different code path.
fn definitely_typed_scoped_name(dep: &str) -> Option<String> {
    let rest = dep.strip_prefix('@')?;
    let (scope, name) = rest.split_once('/')?;
    if scope.is_empty() || name.is_empty() {
        return None;
    }
    Some(format!("{scope}__{name}"))
}

/// M3: per-package variant of `discover_ts_externals`. Reads only the
/// single package's `package.json` (no recursive walk) and searches
/// `{package}/node_modules` plus every ancestor up to `workspace_root`
/// (inclusive) for hoisted deps. Returns roots with `package_id=None`;
/// the caller (locator) stamps ownership.
pub fn discover_ts_externals_scoped(
    workspace_root: &Path,
    package_abs_path: &Path,
) -> Vec<ExternalDepRoot> {
    let Some(declared) = read_single_package_json_deps(package_abs_path) else {
        return Vec::new();
    };
    if declared.is_empty() {
        return Vec::new();
    }

    let node_modules_roots = find_node_modules_with_ancestors(package_abs_path, workspace_root);
    if node_modules_roots.is_empty() {
        debug!("No node_modules dirs discovered for package at {}; skipping TS externals",
            package_abs_path.display());
        return Vec::new();
    }

    let builtins: std::collections::HashSet<&'static str> = [
        "assert", "buffer", "child_process", "cluster", "console", "crypto",
        "dgram", "dns", "domain", "events", "fs", "http", "http2", "https",
        "inspector", "module", "net", "node", "os", "path", "perf_hooks",
        "process", "punycode", "querystring", "readline", "repl", "stream",
        "string_decoder", "timers", "tls", "trace_events", "tty", "url",
        "util", "v8", "vm", "wasi", "worker_threads", "zlib",
    ].into_iter().collect();

    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dep in &declared {
        if builtins.contains(dep.as_str()) { continue }
        if dep.starts_with('@') && !dep.contains('/') { continue }
        if dep.starts_with("@types/") { continue }

        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push(primary); }
            if !dep.starts_with('@') {
                let types_dir = nm_root.join("@types").join(dep);
                if types_dir.is_dir() { pkg_roots.push(types_dir); }
            } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                let types_dir = nm_root.join("@types").join(&escaped);
                if types_dir.is_dir() { pkg_roots.push(types_dir); }
            }
        }
        for pkg_dir in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: "typescript",
                    package_id: None,
                });
            }
        }
    }
    roots
}

/// Read deps from a SINGLE `{dir}/package.json` — no recursive subdir
/// walk, so a workspace-root caller doesn't suck in every sub-package's
/// deps. Node builtins are appended to the result set (they're implicitly
/// available in every JS package). Returns `None` when `package.json` is
/// missing or unparseable.
fn read_single_package_json_deps(dir: &Path) -> Option<std::collections::HashSet<String>> {
    let manifest_path = dir.join("package.json");
    let content = std::fs::read_to_string(&manifest_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let obj = value.as_object()?;
    let mut deps = std::collections::HashSet::new();
    for field in &["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(map) = obj.get(*field).and_then(|v| v.as_object()) {
            for key in map.keys() {
                if key.starts_with('@') {
                    if let Some(scope) = key.split('/').next() {
                        deps.insert(scope.to_string());
                    }
                }
                deps.insert(key.clone());
            }
        }
    }
    Some(deps)
}

/// Probe `{dir}/node_modules` for every directory from `start` walking up
/// (inclusive) to `workspace_root` (inclusive). Handles hoisted workspace
/// layouts where deps live at a shared ancestor, not the individual package.
/// Respects `BEARWISDOM_TS_NODE_MODULES` as an explicit override.
fn find_node_modules_with_ancestors(start: &Path, workspace_root: &Path) -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        let mut out = Vec::new();
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue; }
            if seg.is_dir() && !out.contains(&seg) {
                out.push(seg);
            }
        }
        if !out.is_empty() { return out; }
    }

    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };

    // Package's own node_modules + immediate subdir node_modules.
    push_if_dir(start.join("node_modules"), &mut out);
    if let Ok(entries) = std::fs::read_dir(start) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                )
            {
                continue;
            }
            push_if_dir(entry.path().join("node_modules"), &mut out);
        }
    }

    // Ancestor walk — stop at (and include) workspace_root.
    let mut current = start.parent();
    while let Some(dir) = current {
        push_if_dir(dir.join("node_modules"), &mut out);
        if dir == workspace_root {
            break;
        }
        current = dir.parent();
    }
    out
}

/// Locate node_modules directories for the given project.
///
/// Order of preference:
/// 1. `BEARWISDOM_TS_NODE_MODULES` env override (platform-separated list).
/// 2. `{project_root}/node_modules` (most common).
/// 3. Immediate subdirs of project_root (monorepo pattern: `frontend/`,
///    `packages/`, `apps/`, etc. — same walk shape used for Python venvs).
pub fn find_node_modules(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) {
            out.push(p);
        }
    };

    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() {
                continue;
            }
            push_if_dir(seg, &mut out);
        }
        if !out.is_empty() {
            return out;
        }
    }

    // Root-level node_modules.
    push_if_dir(project_root.join("node_modules"), &mut out);

    // Immediate subdirs — covers `frontend/node_modules`, `apps/web/node_modules`,
    // `packages/foo/node_modules`, etc. Same monorepo-friendly walk shape as
    // `find_python_site_packages`.
    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                )
            {
                continue;
            }
            push_if_dir(entry.path().join("node_modules"), &mut out);
        }
    }

    out
}

/// Walk one TS external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Include `.ts`, `.tsx`, `.d.ts`, `.mts`, `.cts`, `.d.mts`, `.d.cts`.
/// - Skip `.js`/`.jsx`/`.mjs` — type info for those packages lives in
///   sibling `.d.ts` files that we'll pick up anyway.
/// - Skip `node_modules/` subtrees (nested deps).
/// - Skip `__tests__/`, `test/`, `tests/`, `__mocks__/`, `docs/`,
///   `example/`, `examples/`, `_examples/`, `.storybook/`, `fixtures/`.
/// - Skip `*.test.*`, `*.spec.*`, `*.stories.*`, `*.bench.*`, `*.fixture.*`.
///
/// Virtual relative_path is `ext:ts:{package}/{sub_path}`.
pub fn walk_ts_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_ts_dir(&dep.root, &dep.root, dep, &mut out);
    out
}

fn walk_ts_dir(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>) {
    walk_ts_dir_bounded(dir, root, dep, out, 0);
}

fn walk_ts_dir_bounded(dir: &Path, root: &Path, dep: &ExternalDepRoot, out: &mut Vec<WalkedFile>, depth: u32) {
    if depth >= MAX_WALK_DEPTH {
        return;
    }
    let walk_nested = std::env::var_os("BEARWISDOM_TS_WALK_NESTED")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

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
                if name == "node_modules" && !walk_nested {
                    continue;
                }
                if matches!(
                    name,
                    "__tests__"
                        | "__mocks__"
                        | "test"
                        | "tests"
                        | "docs"
                        | "example"
                        | "examples"
                        | "_examples"
                        | "fixtures"
                        | ".storybook"
                        | ".git"
                ) {
                    continue;
                }
            }
            walk_ts_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !is_ts_source_file(name) {
                continue;
            }
            if is_test_or_story_file(name) {
                continue;
            }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:ts:{}/{}", dep.module_path, rel_sub);
            // Tree-sitter TS grammar handles `.d.ts` transparently. `.tsx`
            // needs the TSX-specific grammar, which the language registry
            // routes via the `tsx` language id.
            let language = if name.ends_with(".tsx") {
                "tsx"
            } else {
                "typescript"
            };
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

fn is_ts_source_file(name: &str) -> bool {
    // .d.ts / .d.mts / .d.cts handled by the generic .ts / .mts / .cts suffix match.
    name.ends_with(".ts")
        || name.ends_with(".tsx")
        || name.ends_with(".mts")
        || name.ends_with(".cts")
}

/// Prefix every symbol's `qualified_name` (and `scope_path`) in a parsed
/// TypeScript external file with the owning package name.
///
/// TypeScript declaration files don't carry a package-level scope, so the
/// extractor yields bare qualified names like `Button` or `Button.render`.
/// To make these look up cleanly by `{import_module}.{target}` (which is what
/// the TS Tier 1 resolver tries), rewrite them to `fake-ui.Button` /
/// `fake-ui.Button.render`.
///
/// Mutates the parsed file in place. Idempotent: already-prefixed names are
/// left alone.
pub fn prefix_ts_external_symbols(pf: &mut crate::types::ParsedFile, package: &str) {
    if package.is_empty() {
        return;
    }
    let prefix = format!("{package}.");
    for sym in &mut pf.symbols {
        if !sym.qualified_name.starts_with(&prefix) {
            sym.qualified_name = format!("{prefix}{}", sym.qualified_name);
        }
        sym.scope_path = match sym.scope_path.take() {
            Some(sp) if !sp.starts_with(&prefix) => Some(format!("{prefix}{sp}")),
            Some(sp) => Some(sp),
            None => Some(package.to_string()),
        };
    }
}

fn is_test_or_story_file(name: &str) -> bool {
    // Look for `.test.`, `.spec.`, `.stories.`, `.bench.`, `.fixture.` anywhere
    // before the extension.
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    stem.ends_with(".test")
        || stem.ends_with(".spec")
        || stem.ends_with(".stories")
        || stem.ends_with(".bench")
        || stem.ends_with(".fixture")
        || stem == "test"
        || stem == "index.test"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitely_typed_scoped_escapes() {
        assert_eq!(
            definitely_typed_scoped_name("@tanstack/react-query"),
            Some("tanstack__react-query".to_string())
        );
        assert_eq!(
            definitely_typed_scoped_name("@radix-ui/react-dialog"),
            Some("radix-ui__react-dialog".to_string())
        );
        assert_eq!(definitely_typed_scoped_name("react"), None);
        assert_eq!(definitely_typed_scoped_name("@scope"), None);
        assert_eq!(definitely_typed_scoped_name("@/empty"), None);
    }

    #[test]
    fn ts_source_file_detection() {
        assert!(is_ts_source_file("index.ts"));
        assert!(is_ts_source_file("App.tsx"));
        assert!(is_ts_source_file("index.d.ts"));
        assert!(is_ts_source_file("types.d.mts"));
        assert!(!is_ts_source_file("index.js"));
        assert!(!is_ts_source_file("README.md"));
        assert!(!is_ts_source_file("package.json"));
    }

    #[test]
    fn ts_test_file_detection() {
        assert!(is_test_or_story_file("Foo.test.ts"));
        assert!(is_test_or_story_file("Foo.spec.tsx"));
        assert!(is_test_or_story_file("Button.stories.tsx"));
        assert!(is_test_or_story_file("perf.bench.ts"));
        assert!(!is_test_or_story_file("index.ts"));
        assert!(!is_test_or_story_file("App.tsx"));
        assert!(!is_test_or_story_file("useForm.ts"));
    }

    // ---------------------------------------------------------------
    // M3 — per-package scoped discovery
    // ---------------------------------------------------------------

    #[test]
    fn m3_find_node_modules_walks_ancestors() {
        // Layout: workspace/apps/web/ with workspace/node_modules/ (hoisted).
        // find_node_modules_with_ancestors(apps/web, workspace) must find
        // the workspace-root node_modules via ancestor walk.
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let pkg = ws.join("apps").join("web");
        std::fs::create_dir_all(ws.join("node_modules")).unwrap();
        std::fs::create_dir_all(&pkg).unwrap();

        // Ensure env doesn't leak into the test.
        std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

        let roots = find_node_modules_with_ancestors(&pkg, ws);
        assert!(
            roots.iter().any(|p| p == &ws.join("node_modules")),
            "expected hoisted workspace node_modules, got {roots:?}"
        );
    }

    #[test]
    fn m3_find_node_modules_prefers_package_local() {
        // Layout: workspace/ with both workspace/node_modules and
        // workspace/apps/web/node_modules. Per-package local should
        // appear before the ancestor.
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let pkg = ws.join("apps").join("web");
        std::fs::create_dir_all(ws.join("node_modules")).unwrap();
        std::fs::create_dir_all(pkg.join("node_modules")).unwrap();

        std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

        let roots = find_node_modules_with_ancestors(&pkg, ws);
        let local_idx = roots.iter().position(|p| p == &pkg.join("node_modules"));
        let hoisted_idx = roots.iter().position(|p| p == &ws.join("node_modules"));
        assert!(local_idx.is_some() && hoisted_idx.is_some(),
            "expected both node_modules discovered: {roots:?}");
        assert!(local_idx.unwrap() < hoisted_idx.unwrap(),
            "package-local should precede hoisted: {roots:?}");
    }

    #[test]
    fn m3_read_single_package_json_scoped_to_dir() {
        // The scoped reader must only read the direct package.json, not
        // recursively walk subdirs — otherwise workspace roots would pull
        // in every sub-package's deps.
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dependencies":{"react":"18"},"devDependencies":{"vitest":"1"}}"#,
        ).unwrap();
        // Nested sub-package.json should NOT be read.
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(
            dir.join("sub").join("package.json"),
            r#"{"dependencies":{"axios":"1"}}"#,
        ).unwrap();

        let deps = read_single_package_json_deps(dir).unwrap();
        assert!(deps.contains("react"));
        assert!(deps.contains("vitest"));
        assert!(!deps.contains("axios"), "scoped reader must not recurse into sub/");
    }

    #[test]
    fn m3_discover_ts_externals_scoped_uses_hoisted_node_modules() {
        // Workspace with hoisted node_modules/react/. Package at apps/web
        // declares react. Scoped discovery must find react via ancestor walk.
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let pkg = ws.join("apps").join("web");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"web","dependencies":{"react":"18"}}"#,
        ).unwrap();

        let react_dir = ws.join("node_modules").join("react");
        std::fs::create_dir_all(&react_dir).unwrap();
        std::fs::write(
            react_dir.join("index.d.ts"),
            "export function Component(): any;",
        ).unwrap();

        std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

        let roots = discover_ts_externals_scoped(ws, &pkg);
        assert!(
            roots.iter().any(|r| r.module_path == "react" && r.root == react_dir),
            "expected react root from hoisted node_modules, got {:?}",
            roots.iter().map(|r| (&r.module_path, &r.root)).collect::<Vec<_>>()
        );
    }
}
