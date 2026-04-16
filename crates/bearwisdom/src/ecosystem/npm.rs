// =============================================================================
// ecosystem/npm.rs — npm ecosystem (JS/TS/Vue/Svelte/Angular/Astro/SCSS)
//
// Covers every language whose third-party code lives in `node_modules/`. The
// file-level language detection inside an npm package is already handled by
// the existing walker (`.ts`, `.tsx`, `.d.ts`, `.mts`, `.cts` → TypeScript;
// `.vue` / `.svelte` inside a package route to those plugins via the
// extension registry).
//
// Before this refactor:
//   indexer/externals/typescript.rs — TypeScriptExternalsLocator
//   7 plugins all returned Arc::new(TypeScriptExternalsLocator)
//
// After: one ecosystem, one locator, one walker. The legacy
// `ExternalSourceLocator` trait impl keeps `ecosystem() = "typescript"` so
// DB rows in `package_deps.ecosystem` and existing integration tests
// (full_tests.rs queries `WHERE pd.ecosystem = 'typescript'`) continue to
// work unchanged. Phase 4 migrates the schema and renames.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
};
use crate::indexer::externals::{
    ts_package_from_virtual_path, ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH,
};
use crate::indexer::manifest::npm::NpmManifest;
use crate::indexer::manifest::ManifestReader;
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("npm");

/// Legacy ecosystem tag persisted in `package_deps.ecosystem` and
/// `ExternalDepRoot::ecosystem`. Renamed to "npm" in Phase 4 alongside a
/// DB migration; kept here so no schema change is required in Phase 2.
const LEGACY_ECOSYSTEM_TAG: &str = "typescript";

const MANIFESTS: &[ManifestSpec] = &[];
const LANGUAGES: &[&str] = &[
    "typescript",
    "tsx",
    "javascript",
    "vue",
    "svelte",
    "angular",
    "astro",
    "scss",
];

/// The npm ecosystem. Single locator, single walker, covers every language
/// whose dependencies live in `node_modules/`.
pub struct NpmEcosystem;

// ---------------------------------------------------------------------------
// Ecosystem trait impl (new — authoritative)
// ---------------------------------------------------------------------------

impl Ecosystem for NpmEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Package }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }
    fn manifest_specs(&self) -> &'static [ManifestSpec] { MANIFESTS }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::Any(&[
            EcosystemActivation::ManifestMatch,
            EcosystemActivation::LanguagePresent("typescript"),
            EcosystemActivation::LanguagePresent("tsx"),
            EcosystemActivation::LanguagePresent("javascript"),
            EcosystemActivation::LanguagePresent("vue"),
            EcosystemActivation::LanguagePresent("svelte"),
            EcosystemActivation::LanguagePresent("angular"),
            EcosystemActivation::LanguagePresent("astro"),
        ])
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ts_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_external_root(dep)
    }

    fn post_process_parsed(&self, _dep: &ExternalDepRoot, parsed: &mut crate::types::ParsedFile) {
        if let Some(pkg) = ts_package_from_virtual_path(&parsed.path).map(str::to_string) {
            prefix_ts_external_symbols(parsed, &pkg);
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl — adapter for the indexer pipeline
// until Phase 4 migrates to Ecosystem directly.
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for NpmEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

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

/// Process-wide shared instance used by every npm-consuming plugin.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<NpmEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(NpmEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Node builtins — appear in package.json declared deps but have no on-disk
// source under node_modules. Skipped during walk.
// ---------------------------------------------------------------------------

fn node_builtins() -> std::collections::HashSet<&'static str> {
    [
        "assert", "buffer", "child_process", "cluster", "console", "crypto",
        "dgram", "dns", "domain", "events", "fs", "http", "http2", "https",
        "inspector", "module", "net", "node", "os", "path", "perf_hooks",
        "process", "punycode", "querystring", "readline", "repl", "stream",
        "string_decoder", "timers", "tls", "trace_events", "tty", "url",
        "util", "v8", "vm", "wasi", "worker_threads", "zlib",
    ]
    .into_iter()
    .collect()
}

// ---------------------------------------------------------------------------
// Discovery — project-level
// ---------------------------------------------------------------------------

/// Discover all external TypeScript/JavaScript dependency roots for a project.
///
/// Strategy:
/// 1. Read package.json(s) via `NpmManifest` reader (already walks subdirs
///    and handles dependencies/devDependencies/peerDependencies).
/// 2. Locate node_modules via `BEARWISDOM_TS_NODE_MODULES` env → project-local
///    root → immediate subdirs.
/// 3. For each declared dep, resolve to `node_modules/{name}/` plus the
///    DefinitelyTyped `@types/` fallback for untyped packages.
/// 4. Skip Node builtins.
fn discover_ts_externals(project_root: &Path) -> Vec<ExternalDepRoot> {
    let manifest = NpmManifest;
    let Some(data) = manifest.read(project_root) else { return Vec::new() };
    if data.dependencies.is_empty() { return Vec::new() }

    let node_modules_roots = find_node_modules(project_root);
    if node_modules_roots.is_empty() {
        debug!("No node_modules dirs discovered; skipping npm externals");
        return Vec::new();
    }
    debug!(
        "Probing {} node_modules root(s) for {} declared deps",
        node_modules_roots.len(),
        data.dependencies.len()
    );

    let builtins = node_builtins();
    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for dep in &data.dependencies {
        if builtins.contains(dep.as_str()) { continue }
        if dep.starts_with('@') && !dep.contains('/') { continue }
        if dep.starts_with("@types/") { continue }

        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push(primary) }
            if !dep.starts_with('@') {
                let types_dir = nm_root.join("@types").join(dep);
                if types_dir.is_dir() { pkg_roots.push(types_dir) }
            } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                let types_dir = nm_root.join("@types").join(&escaped);
                if types_dir.is_dir() { pkg_roots.push(types_dir) }
            }
        }

        for pkg_dir in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
            }
        }
    }
    roots
}

/// DefinitelyTyped publishes types for scoped packages at
/// `@types/{scope}__{name}` because npm disallows nested `@` inside a scope
/// path. Returns None for non-scoped names.
fn definitely_typed_scoped_name(dep: &str) -> Option<String> {
    let rest = dep.strip_prefix('@')?;
    let (scope, name) = rest.split_once('/')?;
    if scope.is_empty() || name.is_empty() { return None }
    Some(format!("{scope}__{name}"))
}

// ---------------------------------------------------------------------------
// Discovery — per-package (monorepo M3)
// ---------------------------------------------------------------------------

/// Per-package variant. Reads the single package's `package.json` AND the
/// workspace root's, then merges the dep sets — covers the standard
/// npm/yarn monorepo pattern where root-level devDependencies hold shared
/// test tooling (chai, vitest, jest) that no individual sub-package
/// redeclares. Searches `{package}/node_modules` plus every ancestor up to
/// `workspace_root` (inclusive) for hoisted deps.
fn discover_ts_externals_scoped(
    workspace_root: &Path,
    package_abs_path: &Path,
) -> Vec<ExternalDepRoot> {
    let Some(mut declared) = read_single_package_json_deps(package_abs_path) else {
        return Vec::new();
    };

    if package_abs_path != workspace_root {
        if let Some(root_deps) = read_single_package_json_deps(workspace_root) {
            declared.extend(root_deps);
        }
    }

    if declared.is_empty() { return Vec::new() }

    let node_modules_roots = find_node_modules_with_ancestors(package_abs_path, workspace_root);
    if node_modules_roots.is_empty() {
        debug!(
            "No node_modules dirs discovered for package at {}; skipping npm externals",
            package_abs_path.display()
        );
        return Vec::new();
    }

    let builtins = node_builtins();
    let mut roots = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for dep in &declared {
        if builtins.contains(dep.as_str()) { continue }
        if dep.starts_with('@') && !dep.contains('/') { continue }
        if dep.starts_with("@types/") { continue }

        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push(primary) }
            if !dep.starts_with('@') {
                let types_dir = nm_root.join("@types").join(dep);
                if types_dir.is_dir() { pkg_roots.push(types_dir) }
            } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                let types_dir = nm_root.join("@types").join(&escaped);
                if types_dir.is_dir() { pkg_roots.push(types_dir) }
            }
        }
        for pkg_dir in pkg_roots {
            if seen.insert(pkg_dir.clone()) {
                roots.push(ExternalDepRoot {
                    module_path: dep.clone(),
                    version: String::from("unknown"),
                    root: pkg_dir,
                    ecosystem: LEGACY_ECOSYSTEM_TAG,
                    package_id: None,
                });
            }
        }
    }
    roots
}

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

fn find_node_modules_with_ancestors(start: &Path, workspace_root: &Path) -> Vec<PathBuf> {
    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        let mut out = Vec::new();
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            if seg.is_dir() && !out.contains(&seg) { out.push(seg) }
        }
        if !out.is_empty() { return out }
    }

    let mut out: Vec<PathBuf> = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) { out.push(p) }
    };

    push_if_dir(start.join("node_modules"), &mut out);
    if let Ok(entries) = std::fs::read_dir(start) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                )
            { continue }
            push_if_dir(entry.path().join("node_modules"), &mut out);
        }
    }

    let mut current = start.parent();
    while let Some(dir) = current {
        push_if_dir(dir.join("node_modules"), &mut out);
        if dir == workspace_root { break }
        current = dir.parent();
    }
    out
}

fn find_node_modules(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_dir = |p: PathBuf, out: &mut Vec<PathBuf>| {
        if p.is_dir() && !out.contains(&p) { out.push(p) }
    };

    if let Some(raw) = std::env::var_os("BEARWISDOM_TS_NODE_MODULES") {
        for seg in std::env::split_paths(&raw) {
            if seg.as_os_str().is_empty() { continue }
            push_if_dir(seg, &mut out);
        }
        if !out.is_empty() { return out }
    }

    push_if_dir(project_root.join("node_modules"), &mut out);

    if let Ok(entries) = std::fs::read_dir(project_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) { continue }
            let name = entry.file_name();
            let name_lossy = name.to_string_lossy();
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                )
            { continue }
            push_if_dir(entry.path().join("node_modules"), &mut out);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Walk
// ---------------------------------------------------------------------------

/// Walk one npm external dep root and emit `WalkedFile` entries.
///
/// File filtering rules:
/// - Include `.ts`, `.tsx`, `.d.ts`, `.mts`, `.cts`, `.d.mts`, `.d.cts`.
/// - Skip `.js`/`.jsx`/`.mjs` (type info lives in sibling `.d.ts`).
/// - Skip nested `node_modules/` unless `BEARWISDOM_TS_WALK_NESTED=1`.
/// - Skip test/story/example/fixture dirs and files.
///
/// Virtual relative_path is `ext:ts:{package}/{sub_path}`.
fn walk_ts_external_root(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let mut out = Vec::new();
    walk_ts_dir_bounded(&dep.root, &dep.root, dep, &mut out, 0);
    out
}

fn walk_ts_dir_bounded(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    out: &mut Vec<WalkedFile>,
    depth: u32,
) {
    if depth >= MAX_WALK_DEPTH { return }
    let walk_nested = std::env::var_os("BEARWISDOM_TS_WALK_NESTED")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "node_modules" && !walk_nested { continue }
                if matches!(
                    name,
                    "__tests__" | "__mocks__" | "test" | "tests" | "docs"
                        | "example" | "examples" | "_examples" | "fixtures"
                        | ".storybook" | ".git"
                ) { continue }
            }
            walk_ts_dir_bounded(&path, root, dep, out, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !is_ts_source_file(name) { continue }
            if is_test_or_story_file(name) { continue }

            let rel_sub = match path.strip_prefix(root) {
                Ok(p) => p.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let virtual_path = format!("ext:ts:{}/{}", dep.module_path, rel_sub);
            let language = if name.ends_with(".tsx") { "tsx" } else { "typescript" };
            out.push(WalkedFile {
                relative_path: virtual_path,
                absolute_path: path,
                language,
            });
        }
    }
}

fn is_ts_source_file(name: &str) -> bool {
    name.ends_with(".ts")
        || name.ends_with(".tsx")
        || name.ends_with(".mts")
        || name.ends_with(".cts")
}

fn is_test_or_story_file(name: &str) -> bool {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    stem.ends_with(".test")
        || stem.ends_with(".spec")
        || stem.ends_with(".stories")
        || stem.ends_with(".bench")
        || stem.ends_with(".fixture")
        || stem == "test"
        || stem == "index.test"
}

// ---------------------------------------------------------------------------
// Post-process: prefix declaration-file symbols with their package name
// ---------------------------------------------------------------------------

/// Prefix every symbol's `qualified_name` (and `scope_path`) in a parsed
/// TypeScript external file with the owning package name.
///
/// TypeScript declaration files don't carry a package-level scope, so the
/// extractor yields bare qualified names like `Button`. Rewrite them to
/// `fake-ui.Button` so the TS resolver's `{import_module}.{target}` lookup
/// matches. Idempotent: already-prefixed names are left alone.
fn prefix_ts_external_symbols(pf: &mut crate::types::ParsedFile, package: &str) {
    if package.is_empty() { return }
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_identity() {
        let n = NpmEcosystem;
        assert_eq!(n.id(), ID);
        assert_eq!(Ecosystem::kind(&n), EcosystemKind::Package);
        assert!(Ecosystem::languages(&n).contains(&"typescript"));
        assert!(Ecosystem::languages(&n).contains(&"javascript"));
        assert!(Ecosystem::languages(&n).contains(&"vue"));
        assert!(Ecosystem::languages(&n).contains(&"svelte"));
    }

    #[test]
    fn legacy_locator_string_unchanged() {
        // Keep "typescript" to avoid schema/test churn in Phase 2.
        assert_eq!(ExternalSourceLocator::ecosystem(&NpmEcosystem), "typescript");
    }

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

    // -----------------------------------------------------------------
    // M3 — per-package scoped discovery (migrated from typescript.rs)
    // -----------------------------------------------------------------

    #[test]
    fn m3_find_node_modules_walks_ancestors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let pkg = ws.join("apps").join("web");
        std::fs::create_dir_all(ws.join("node_modules")).unwrap();
        std::fs::create_dir_all(&pkg).unwrap();
        std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

        let roots = find_node_modules_with_ancestors(&pkg, ws);
        assert!(
            roots.iter().any(|p| p == &ws.join("node_modules")),
            "expected hoisted workspace node_modules, got {roots:?}"
        );
    }

    #[test]
    fn m3_find_node_modules_prefers_package_local() {
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
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        std::fs::write(
            dir.join("package.json"),
            r#"{"dependencies":{"react":"18"},"devDependencies":{"vitest":"1"}}"#,
        ).unwrap();
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
            "expected react root from hoisted node_modules"
        );
    }

    #[test]
    fn m3_discover_ts_externals_scoped_merges_workspace_root_deps() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path();
        let pkg = ws.join("hooks");
        std::fs::create_dir_all(&pkg).unwrap();

        std::fs::write(
            ws.join("package.json"),
            r#"{"name":"preact","devDependencies":{"chai":"5","vitest":"2"}}"#,
        ).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"name":"preact-hooks","dependencies":{"preact":"*"}}"#,
        ).unwrap();

        let chai_dir = ws.join("node_modules").join("@types").join("chai");
        std::fs::create_dir_all(&chai_dir).unwrap();
        std::fs::write(chai_dir.join("index.d.ts"), "export function assert(x: any): void;").unwrap();

        let vitest_dir = ws.join("node_modules").join("vitest");
        std::fs::create_dir_all(&vitest_dir).unwrap();
        std::fs::write(vitest_dir.join("index.d.ts"), "export function describe(n: string, f: () => void): void;").unwrap();

        let preact_dir = ws.join("node_modules").join("preact");
        std::fs::create_dir_all(&preact_dir).unwrap();
        std::fs::write(preact_dir.join("index.d.ts"), "export function h(): any;").unwrap();

        std::env::remove_var("BEARWISDOM_TS_NODE_MODULES");

        let roots = discover_ts_externals_scoped(ws, &pkg);
        assert!(roots.iter().any(|r| r.module_path == "chai"),
            "expected chai from workspace root devDeps");
        assert!(roots.iter().any(|r| r.module_path == "vitest"),
            "expected vitest from workspace root devDeps");
        assert!(roots.iter().any(|r| r.module_path == "preact"),
            "expected preact from sub-package deps");
    }

    #[allow(dead_code)]
    fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
        shared_locator()
    }
}
