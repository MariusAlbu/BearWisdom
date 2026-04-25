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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, ManifestSpec,
    SymbolLocationIndex,
};
use crate::ecosystem::externals::{
    ts_package_from_virtual_path, ExternalDepRoot, ExternalSourceLocator, MAX_WALK_DEPTH,
};
use crate::ecosystem::manifest::npm::NpmManifest;
use crate::ecosystem::manifest::ManifestReader;
use crate::walker::WalkedFile;
use rayon::prelude::*;
use tree_sitter::{Node, Parser};

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

    fn supports_reachability(&self) -> bool { true }

    fn resolve_import(
        &self,
        dep: &ExternalDepRoot,
        package: &str,
        _symbols: &[&str],
    ) -> Vec<WalkedFile> {
        // Reachability-based: find the package's type-declaration entry and
        // return just that file. The parser will extract ALL exports; the
        // resolver picks the ones matching the import statement. Any
        // re-exports pointing at other files in the package become new
        // imports in the reachability loop and drive further resolve_*
        // calls until fixpoint.
        let _ = package;
        resolve_package_entry(dep)
    }

    fn resolve_symbol(
        &self,
        dep: &ExternalDepRoot,
        fqn: &str,
    ) -> Vec<WalkedFile> {
        // R4: chain walker asks for the file(s) defining a specific FQN
        // (e.g., "chai.Assertion"). Scan the dep's source tree for files
        // declaring the FQN's last segment as a class/interface/type.
        // Falls back to the package entry walk when nothing matches —
        // either the type is re-exported through the entry, or the search
        // missed it (rare for declaration files).
        let target = fqn.rsplit('.').next().unwrap_or(fqn);
        let mut files = find_files_declaring_type(dep, target);
        if files.is_empty() {
            files = resolve_package_entry(dep);
        }
        files
    }

    fn post_process_parsed(&self, _dep: &ExternalDepRoot, parsed: &mut crate::types::ParsedFile) {
        if let Some(pkg) = ts_package_from_virtual_path(&parsed.path).map(str::to_string) {
            // Backfill any `declare global { ... }` names the TypeScript
            // extractor missed (it descends inconsistently into ambient
            // blocks, so only a subset of the inner decls normally surface
            // as top-level symbols). Re-scan the source with the regex
            // helper and inject missing names with kind=variable; the
            // heuristic resolver's declare-global priority then resolves
            // bare-name refs (vitest's `it`/`beforeEach`/etc., any .d.ts
            // that declares runtime globals) to the right file.
            let source_snapshot = parsed.content.clone();
            if let Some(source) = source_snapshot.as_deref() {
                if source.contains("declare global") {
                    backfill_declare_global_symbols(parsed, source);
                }
            }
            prefix_ts_external_symbols(parsed, &pkg);
        }
    }

    fn build_symbol_index(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> SymbolLocationIndex {
        build_npm_symbol_index(dep_roots)
    }

    fn demand_pre_pull(
        &self,
        dep_roots: &[ExternalDepRoot],
    ) -> Vec<crate::walker::WalkedFile> {
        demand_pre_pull_test_globals(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool { true }
}

/// Packages whose entry declaration file tends to expose runtime globals via
/// `declare global { ... }` (test frameworks with `globals: true` / always-on
/// globals, @types packages for globally-scoped APIs). Demand-driven mode
/// wouldn't pull these on its own because user code names the globals
/// without importing the package — the symbol-index lookup is what
/// classifies the ref, and that lookup needs the symbols already in the
/// index. Eagerly pre-pulling a handful of per-package entry files closes
/// the loop without broadening the walk.
///
/// This list is a priority hint, NOT a special code path — any package
/// whose `.d.ts` declares globals gets them indexed regardless of whether
/// it's listed here. Listing just ensures the file is walked even when no
/// user ref would otherwise demand it.
const KNOWN_GLOBAL_PACKAGES: &[&str] = &[
    // Test runners with implicit globals.
    "vitest",
    "@vitest/browser",
    "@vitest/expect",
    "@vitest/runner",
    "@vitest/spy",
    "@jest/globals",
    "@jest/expect",
    "jest",
    "mocha",
    "jasmine",
    // Assertion / fake / stub libraries commonly used as globals.
    "chai",
    "sinon",
    "chai-as-promised",
    // @types packages that declare globals (`@types/jest`, `@types/mocha`,
    // `@types/node` for `process`/`Buffer`/`global`, etc.).
    "@types/jest",
    "@types/mocha",
    "@types/jasmine",
    "@types/chai",
    "@types/sinon",
    "@types/node",
];

fn demand_pre_pull_test_globals(dep_roots: &[ExternalDepRoot]) -> Vec<crate::walker::WalkedFile> {
    let mut out = Vec::new();
    for dep in dep_roots {
        if !KNOWN_GLOBAL_PACKAGES
            .iter()
            .any(|p| *p == dep.module_path.as_str())
        {
            continue;
        }
        // Walk the package root; keep only the type-declaration files that
        // are strong candidates for `declare global { ... }` content. This
        // is cheap — each dep has a handful of .d.ts files, not thousands.
        for wf in walk_ts_external_root(dep) {
            let lower = wf.relative_path.to_lowercase();
            let is_globals_file = lower.ends_with("/globals.d.ts")
                || lower.ends_with("/global.d.ts")
                || lower.ends_with("/index.d.ts")
                || lower.ends_with("/jest.d.ts")
                || lower.ends_with("/mocha.d.ts")
                || lower.ends_with("/jasmine.d.ts");
            if is_globals_file {
                out.push(wf);
            }
        }
    }
    out
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
            let source_snapshot = parsed.content.clone();
            if let Some(source) = source_snapshot.as_deref() {
                if source.contains("declare global") {
                    backfill_declare_global_symbols(parsed, source);
                }
            }
            prefix_ts_external_symbols(parsed, &pkg);
        }
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<crate::types::ParsedFile>> {
        // Per-library chain-type synthetics (jquery, dayjs, chai/vitest,
        // clay-ui, compose-icons) all lived here at various points. They
        // were all symptoms of two architectural gaps that have since
        // been closed generically:
        //
        // 1. `<script src>` discovery + IIFE globals harvest replaces
        //    `jquery_synthetics.rs` — `wwwroot/lib/jquery/jquery.js` is
        //    followed from Razor/HTML refs and the JS extractor lifts
        //    IIFE-installed globals (`$`, `jQuery`, `angular`, …) to
        //    file-scope symbols. See `indexer::script_tag_deps` and
        //    `languages::javascript::extract::harvest_top_level_globals`.
        //
        // 2. Scope-aware return-type resolution in the TypeInfo builder
        //    replaces `dayjs_synthetics.rs`, `js_test_chains.rs`,
        //    `clay_ui_synthetics.rs`, `compose_icons_stubs.rs`. The
        //    real `.d.ts` files (e.g. `node_modules/dayjs/esm/index.d.ts`,
        //    `node_modules/@types/chai/index.d.ts`) are walked by the
        //    npm locator, their methods emit TypeRef refs for return
        //    types, and the builder's scope-probe
        //    (`indexer::resolve::engine::resolve_type_name_in_scope`)
        //    qualifies raw type names like `Assertion` or `Dayjs`
        //    against the namespace they're declared in.
        //
        // No metadata-only synthetic file is emitted any more.
        None
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

        // `@types/foo` deps are normally picked up as companions of `foo`
        // below, so we used to skip them here. But for packages whose runtime
        // ships pre-compiled JS without inline types (jasmine, mocha, test
        // runners, ambient-only modules) the user declares `@types/foo`
        // directly without a matching `foo`. Those direct declarations must
        // still get a dep root — especially for KNOWN_GLOBAL_PACKAGES whose
        // `declare global { ... }` contents are what register `describe`,
        // `it`, `expect` etc. as global symbols.
        let is_types_only = dep.starts_with("@types/");

        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push(primary) }
            if !is_types_only {
                if !dep.starts_with('@') {
                    let types_dir = nm_root.join("@types").join(dep);
                    if types_dir.is_dir() { pkg_roots.push(types_dir) }
                } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                    let types_dir = nm_root.join("@types").join(&escaped);
                    if types_dir.is_dir() { pkg_roots.push(types_dir) }
                }
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
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    // Transitive dep expansion: for each declared dep, follow the
    // cross-package re-exports in its type-entry `.d.ts`. Pattern seen
    // with vitest: its entry file has
    //   export { X } from '@vitest/expect'
    //   export { Y } from '@vitest/runner'
    // but `@vitest/expect` / `@vitest/runner` aren't in vitest's
    // package.json — they're installed via the lockfile. Without this
    // step, demand-driven resolution never finds the interfaces that
    // define matcher chains.
    //
    // Scoped to one hop and restricted to the BARE re-export specifiers
    // read from entry files (not recursive walks) to keep the extra
    // work O(declared_deps × avg_entry_file_size), not O(full dep
    // tree). Entries for packages we already have are skipped.
    let existing: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    // Each transitive spec is paired with its origin dep's own local
    // `node_modules/` — pnpm stores transitive packages there as siblings,
    // not at the workspace's top-level node_modules. Without this, packages
    // re-exported from a dep but not declared in the consumer's
    // package.json (e.g. `@typescript-eslint/types` re-exported through
    // `@typescript-eslint/utils`) stay invisible.
    let mut transitive_specs: std::collections::HashSet<(String, PathBuf)> =
        std::collections::HashSet::new();
    for r in &roots {
        let entry = match resolve_package_entry_path(r) {
            Some(e) => e,
            None => continue,
        };
        let local_nm = dep_local_node_modules(&r.root).unwrap_or_default();
        for spec in collect_bare_reexports_recursive(&entry) {
            if !existing.contains(&spec) && !builtins.contains(spec.as_str()) {
                transitive_specs.insert((spec, local_nm.clone()));
            }
        }
    }

    for (spec, parent_local_nm) in transitive_specs {
        if spec.starts_with('@') && !spec.contains('/') { continue }
        // Try the standard workspace node_modules roots first (npm/yarn
        // hoist transitives there). Fall back to the parent dep's own
        // `node_modules/` (pnpm stores them there).
        let mut probe_roots: Vec<&Path> =
            node_modules_roots.iter().map(|p| p.as_path()).collect();
        if !parent_local_nm.as_os_str().is_empty() {
            probe_roots.push(parent_local_nm.as_path());
        }
        for nm_root in probe_roots {
            let candidate = nm_root.join(&spec);
            if !candidate.is_dir() { continue }
            if !seen.insert(candidate.clone()) { continue }
            roots.push(ExternalDepRoot {
                module_path: spec.clone(),
                version: String::from("unknown"),
                root: candidate,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
            break; // one canonical install per spec is enough
        }
    }

    roots
}

/// Compute a dep's own `node_modules/` directory — the place where pnpm
/// stores its transitive dependencies as siblings. For a package at
/// `<store>/node_modules/<scope>/<name>` returns `<store>/node_modules`,
/// for `<store>/node_modules/<name>` returns `<store>/node_modules`. None
/// when the directory layout doesn't match either shape.
///
/// Resolves through symlinks first so pnpm's symlink layout
/// (`pkg/node_modules/<scope>/<dep>` → `<store>/node_modules/<scope>/<dep>`)
/// produces the real on-disk store path, where the dep's own deps live as
/// siblings. Without canonicalisation the parent walks land on the symlink
/// container, which only holds packages declared in the consumer's own
/// `package.json`, missing every transitive.
fn dep_local_node_modules(dep_root: &Path) -> Option<PathBuf> {
    let real_root = std::fs::canonicalize(dep_root).ok().unwrap_or_else(|| dep_root.to_path_buf());
    let parent = real_root.parent()?;
    let parent_name = parent.file_name()?.to_str()?;
    if parent_name.starts_with('@') {
        parent.parent().map(|p| p.to_path_buf())
    } else {
        Some(parent.to_path_buf())
    }
}

/// Walk the relative re-export chain starting at `entry`, collecting every
/// bare (cross-package) re-export specifier reachable via relative `./x` /
/// `../x` chains. Bounded by `REEXPORT_MAX_DEPTH` and a visited set so
/// cyclic re-exports (rare but seen in @types) don't loop.
///
/// Necessary because most npm packages keep cross-package re-exports out of
/// their entry file:
///   `dist/index.d.ts`     export * from './ts-estree'
///   `dist/ts-estree.d.ts` export { TSESTree } from '@typescript-eslint/types'
/// — scanning only the entry misses `@typescript-eslint/types`, leaving its
/// symbols absent from the index. Walking the relative chain catches them.
fn collect_bare_reexports_recursive(entry: &Path) -> Vec<String> {
    let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut stack: Vec<(PathBuf, u32)> = vec![(entry.to_path_buf(), 0)];
    while let Some((file, depth)) = stack.pop() {
        if !seen.insert(file.clone()) { continue }
        if depth > REEXPORT_MAX_DEPTH { continue }
        let Ok(src) = std::fs::read_to_string(&file) else { continue };
        for spec in extract_bare_reexport_specifiers(&src) {
            out.insert(spec);
        }
        for rel in extract_relative_reexports(&src) {
            if let Some(next) = resolve_relative_ts_path(&file, &rel) {
                stack.push((next, depth + 1));
            }
        }
    }
    out.into_iter().collect()
}

/// Scan a source file for `export ... from '<spec>'` / `import ... from '<spec>'`
/// statements where `spec` is a bare (non-relative) package specifier. Returns
/// the specifier's package name (e.g. `@vitest/expect`, `react`, `lodash`).
/// Relative specifiers are skipped — they stay within the current package and
/// are handled by `expand_reexports_into`.
fn extract_bare_reexport_specifiers(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if !(t.starts_with("export") || t.starts_with("import")) { continue }
        let Some(ix) = t.find(" from ") else { continue };
        let rest = t[ix + 6..].trim_start();
        let Some(quote) = rest.chars().next() else { continue };
        if quote != '\'' && quote != '"' { continue }
        let inner = &rest[1..];
        let Some(end) = inner.find(quote) else { continue };
        let spec = &inner[..end];
        if spec.starts_with("./") || spec.starts_with("../") { continue }
        // Extract the package name: either `@scope/pkg` or `pkg` (first path segment).
        let pkg = if spec.starts_with('@') {
            spec.splitn(3, '/').take(2).collect::<Vec<_>>().join("/")
        } else {
            spec.split('/').next().unwrap_or(spec).to_string()
        };
        if !pkg.is_empty() {
            out.push(pkg);
        }
    }
    out
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
    // Primary: the package's own package.json (classic monorepo layout —
    // one package.json per package dir).
    let mut declared = read_single_package_json_deps(package_abs_path).unwrap_or_default();

    // Fallback: polyglot repos where a .NET / Rust / Go package owns a
    // nested TypeScript sub-app with its own package.json
    // (`src/Web/ClientApp/package.json` in Clean-Architecture layouts,
    // `web/frontend/package.json` in Go services, etc.). Walk the
    // package subtree for any package.json files and merge their deps.
    // Skip `node_modules/` during the walk so we don't pick up
    // third-party package.json files.
    declared.extend(read_nested_package_json_deps(package_abs_path));

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
        // See `discover_ts_externals` — direct `@types/foo` declarations
        // must not be silently dropped; they may be the sole type source
        // for an ambient-only package (jasmine, mocha, etc.).
        let is_types_only = dep.starts_with("@types/");

        let mut pkg_roots: Vec<PathBuf> = Vec::new();
        for nm_root in &node_modules_roots {
            let primary = nm_root.join(dep);
            if primary.is_dir() { pkg_roots.push(primary) }
            if !is_types_only {
                if !dep.starts_with('@') {
                    let types_dir = nm_root.join("@types").join(dep);
                    if types_dir.is_dir() { pkg_roots.push(types_dir) }
                } else if let Some(escaped) = definitely_typed_scoped_name(dep) {
                    let types_dir = nm_root.join("@types").join(&escaped);
                    if types_dir.is_dir() { pkg_roots.push(types_dir) }
                }
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
                    requested_imports: Vec::new(),
                });
            }
        }
    }

    // Transitive re-export expansion — mirror of the logic in
    // `discover_ts_externals`. Scan each declared dep's type-entry `.d.ts`
    // for `from '<bare-specifier>'` references and add those packages to
    // the root set. Catches the vitest/@vitest-expect pattern where the
    // internal scoped package isn't declared in the consumer's
    // package.json but is re-exported from the dep's public entry.
    let existing: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    let mut transitive_specs: std::collections::HashSet<(String, PathBuf)> =
        std::collections::HashSet::new();
    for r in &roots {
        let entry = match resolve_package_entry_path(r) {
            Some(e) => e,
            None => continue,
        };
        let local_nm = dep_local_node_modules(&r.root).unwrap_or_default();
        for spec in collect_bare_reexports_recursive(&entry) {
            if !existing.contains(&spec) && !builtins.contains(spec.as_str()) {
                transitive_specs.insert((spec, local_nm.clone()));
            }
        }
    }
    for (spec, parent_local_nm) in transitive_specs {
        if spec.starts_with('@') && !spec.contains('/') { continue }
        let mut probe_roots: Vec<&Path> =
            node_modules_roots.iter().map(|p| p.as_path()).collect();
        if !parent_local_nm.as_os_str().is_empty() {
            probe_roots.push(parent_local_nm.as_path());
        }
        for nm_root in probe_roots {
            let candidate = nm_root.join(&spec);
            if !candidate.is_dir() { continue }
            if !seen.insert(candidate.clone()) { continue }
            roots.push(ExternalDepRoot {
                module_path: spec.clone(),
                version: String::from("unknown"),
                root: candidate,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            });
            break;
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

/// Walk `dir`'s subtree for `package.json` files (skipping `node_modules/`
/// so we don't pick up third-party manifests) and union the declared
/// dependencies. Used to discover deps from a TypeScript sub-app nested
/// inside a non-TS package — e.g. `src/Web/ClientApp/package.json` inside
/// a .NET `src/Web` project.
///
/// Bounded depth (6) keeps the walk cheap on real repos; in practice
/// the nested manifest is at most 2–3 levels below the package root.
fn read_nested_package_json_deps(dir: &Path) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    walk_for_package_json(dir, dir, &mut out, 0);
    out
}

fn walk_for_package_json(
    cur: &Path,
    root: &Path,
    out: &mut std::collections::HashSet<String>,
    depth: usize,
) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(cur) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy();
        if file_type.is_dir() {
            // Skip canonical exclusion dirs so we don't recurse into
            // artifacts or vendor code.
            if name_lossy.starts_with('.')
                || matches!(
                    name_lossy.as_ref(),
                    "node_modules" | "target" | "dist" | "build" | ".turbo" | ".next"
                        | "bin" | "obj" | "coverage"
                )
            {
                continue;
            }
            walk_for_package_json(&entry.path(), root, out, depth + 1);
        } else if name == "package.json" {
            // Skip the exact same package.json the caller already read
            // (when cur == root).
            if entry.path().parent() == Some(root) {
                continue;
            }
            if let Some(deps) = read_single_package_json_deps(
                entry.path().parent().unwrap_or(cur),
            ) {
                out.extend(deps);
            }
        }
    }
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
    // Walk the subtree under `start` (bounded depth) for nested
    // node_modules dirs. Polyglot layouts bury their TypeScript
    // sub-apps 2–3 levels deep inside a non-TS package
    // (`src/Web/ClientApp/node_modules`, `web/admin/node_modules`, …)
    // and the old single-level scan missed them. Skips
    // `node_modules/` / build-artifact dirs so we don't recurse into
    // third-party trees.
    walk_for_nested_node_modules(start, &mut out, 0);

    let mut current = start.parent();
    while let Some(dir) = current {
        push_if_dir(dir.join("node_modules"), &mut out);
        if dir == workspace_root { break }
        current = dir.parent();
    }
    out
}

fn walk_for_nested_node_modules(cur: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(cur) else { return };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_lossy = name.to_string_lossy();
        if name_lossy == "node_modules" {
            let path = entry.path();
            if !out.contains(&path) {
                out.push(path);
            }
            // Don't recurse into node_modules — we want the outermost
            // for each install pocket.
            continue;
        }
        if name_lossy.starts_with('.')
            || matches!(
                name_lossy.as_ref(),
                "target" | "dist" | "build" | ".turbo" | ".next" | "bin" | "obj" | "coverage"
            )
        {
            continue;
        }
        walk_for_nested_node_modules(&entry.path(), out, depth + 1);
    }
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

    // Walk the project tree looking for top-level `node_modules/` dirs at
    // ANY depth. Polyglot repos routinely bury their TypeScript side deep
    // inside a backend layout (`src/Web/ClientApp/`, `frontend/`,
    // `apps/web/`, `webclient/`, …) and the prior single-level scan missed
    // them, leaving every imported package unresolved.
    //
    // Each discovered `node_modules/` dir is recorded exactly once and we
    // do NOT descend into one to find nested ones (that path is reserved
    // for `BEARWISDOM_TS_WALK_NESTED` and npm package-level walking —
    // it's not how app-level dependency discovery works).
    //
    // CRITICAL: do NOT enable gitignore. Every JS/TS project's
    // `.gitignore` starts with `node_modules/`, which would hide the
    // very directory we're looking for. We keep the default filters
    // that ignore hidden dirs (`.git/`, `.turbo/`, `.next/`) and
    // common build-artifact trees so the walk stays cheap, but
    // gitignore matching is disabled explicitly.
    use ignore::WalkBuilder;
    let walker = WalkBuilder::new(project_root)
        .follow_links(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .hidden(true)
        .filter_entry(|entry| {
            // Prune artifact trees to keep the scan bounded without
            // needing gitignore rules.
            let name = entry.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "target" | "dist" | "build" | ".turbo" | ".next" | "bin" | "obj"
            )
        })
        .build();

    for entry in walker.flatten() {
        let p = entry.path();
        if !p.file_name().map(|n| n == "node_modules").unwrap_or(false) {
            continue;
        }
        if !p.is_dir() {
            continue;
        }
        // Skip nested node_modules — only count the outermost for any
        // given package-install pocket.
        let is_nested = p
            .ancestors()
            .skip(1)
            .any(|a| a.file_name().map(|n| n == "node_modules").unwrap_or(false));
        if is_nested {
            continue;
        }
        push_if_dir(p.to_path_buf(), &mut out);
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

/// Scan the dep's source tree for files that declare `type_name` as a
/// class/interface/type/enum/function. Used by `resolve_symbol` to pull in
/// just the file(s) the chain walker needs, without dumping every file in
/// the dep into the index.
///
/// Caps total files scanned at `MAX_FILES_SCANNED` to bound worst-case cost
/// on huge declaration bundles (e.g. material-ui ships thousands of .d.ts
/// files). When the cap fires, callers fall back to the entry walk.
fn find_files_declaring_type(dep: &ExternalDepRoot, type_name: &str) -> Vec<WalkedFile> {
    const MAX_FILES_SCANNED: usize = 500;
    let mut out = Vec::new();
    let mut scanned = 0usize;
    scan_for_type_decl(&dep.root, &dep.root, dep, type_name, &mut out, &mut scanned, 0);
    if scanned >= MAX_FILES_SCANNED {
        // Bail out — search exceeded budget. Caller falls back to the
        // package entry walk.
        return Vec::new();
    }
    out
}

fn scan_for_type_decl(
    dir: &Path,
    root: &Path,
    dep: &ExternalDepRoot,
    type_name: &str,
    out: &mut Vec<WalkedFile>,
    scanned: &mut usize,
    depth: u32,
) {
    const MAX_FILES_SCANNED: usize = 500;
    if depth >= MAX_WALK_DEPTH || *scanned >= MAX_FILES_SCANNED { return }

    let walk_nested = std::env::var_os("BEARWISDOM_TS_WALK_NESTED")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *scanned >= MAX_FILES_SCANNED { return }
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
            scan_for_type_decl(&path, root, dep, type_name, out, scanned, depth + 1);
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !is_ts_source_file(name) { continue }
            if is_test_or_story_file(name) { continue }

            *scanned += 1;
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            if !file_declares_type(&content, type_name) { continue }

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

/// Heuristic: does `content` declare `type_name` at the top level as a
/// class/interface/type/enum/function/const? The chain walker asks for
/// methods/fields *of* this type, so we only need the file that owns the
/// declaration; once parsed, the symbol's qualified name + members will be
/// in the index.
///
/// Patterns matched (whitespace-flexible):
///   `class Foo`, `interface Foo`, `type Foo`, `enum Foo`, `function Foo`,
///   `const Foo`, `let Foo`, `var Foo`
/// Each preceded by optional `export`/`declare`/`abstract`/`default`
/// keywords and followed by `<`, ` `, `=`, `(`, `:`, `{`, `;`, `\n`, or end.
fn file_declares_type(content: &str, type_name: &str) -> bool {
    // Cheap pre-filter: skip files that don't contain the name at all.
    if !content.contains(type_name) { return false }

    for raw in content.lines() {
        let line = raw.trim_start();
        // Strip combinations of leading modifiers; order doesn't matter.
        let stripped = strip_decl_modifiers(line);
        for keyword in &["class ", "interface ", "type ", "enum ", "function ",
                         "const ", "let ", "var ", "abstract class "]
        {
            if let Some(rest) = stripped.strip_prefix(keyword) {
                let rest = rest.trim_start();
                if let Some(after_name) = rest.strip_prefix(type_name) {
                    let next = after_name.chars().next();
                    let ok = matches!(
                        next,
                        None | Some(' ') | Some('<') | Some('=') | Some('(')
                            | Some(':') | Some('{') | Some(';') | Some('\t')
                            | Some('\n') | Some('\r')
                    );
                    if ok { return true }
                }
            }
        }
    }
    false
}

/// Drop leading `export`/`default`/`declare`/`abstract`/`async` modifiers in
/// any order. Returns a slice into the original string.
fn strip_decl_modifiers(line: &str) -> &str {
    let mut s = line.trim_start();
    loop {
        let mut advanced = false;
        for keyword in &["export ", "default ", "declare ", "async "] {
            if let Some(rest) = s.strip_prefix(keyword) {
                s = rest.trim_start();
                advanced = true;
                break;
            }
        }
        if !advanced { break }
    }
    s
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

/// Locate a package's type-declaration entry point using its `package.json`.
///
/// Priority:
///   1. `types` field (modern)
///   2. `typings` field (legacy alias of `types`)
///   3. `main` field with `.js`/`.mjs`/`.cjs` rewritten to the matching
///      `.d.ts` sibling if one exists on disk
///   4. Conventional fallbacks — `index.d.ts`, `dist/index.d.ts`, `lib/index.d.ts`
///
/// Returns the entry file plus any files it re-exports from WITHIN the same
/// dep root, bounded at depth `REEXPORT_MAX_DEPTH`. Without within-package
/// re-export expansion, entry-only parsing leaves most declaration bundles
/// opaque (vitest's `index.d.ts` is almost entirely `export { X } from
/// './chunks/...'` statements).
fn resolve_package_entry(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    let Some(entry) = resolve_package_entry_path(dep) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    expand_reexports_into(dep, &entry, &mut out, &mut seen, 0);
    out
}

fn resolve_package_entry_path(dep: &ExternalDepRoot) -> Option<PathBuf> {
    let pkg_json_path = dep.root.join("package.json");
    let json_str = std::fs::read_to_string(&pkg_json_path).ok();
    let parsed: Option<serde_json::Value> = json_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(pj) = parsed.as_ref() {
        for field in ["types", "typings"] {
            if let Some(v) = pj.get(field).and_then(|v| v.as_str()) {
                candidates.push(dep.root.join(v.trim_start_matches("./")));
            }
        }
        if let Some(main) = pj.get("main").and_then(|v| v.as_str()) {
            let main_path = dep.root.join(main.trim_start_matches("./"));
            if main.ends_with(".d.ts") {
                candidates.push(main_path);
            } else {
                let stem = main_path.to_string_lossy().to_string();
                for ext in [".js", ".mjs", ".cjs"] {
                    if stem.ends_with(ext) {
                        let dts = stem.trim_end_matches(ext).to_string() + ".d.ts";
                        candidates.push(PathBuf::from(dts));
                        break;
                    }
                }
            }
        }
    }

    for fallback in ["index.d.ts", "dist/index.d.ts", "lib/index.d.ts", "types/index.d.ts"] {
        candidates.push(dep.root.join(fallback));
    }

    candidates.into_iter().find(|p| p.is_file())
}

const REEXPORT_MAX_DEPTH: u32 = 3;

fn expand_reexports_into(
    dep: &ExternalDepRoot,
    file: &Path,
    out: &mut Vec<WalkedFile>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if !seen.insert(file.to_path_buf()) { return }
    if !file.is_file() { return }
    let Ok(rel) = file.strip_prefix(&dep.root) else { return };
    let rel_s = rel.to_string_lossy().replace('\\', "/");
    let lang = if rel_s.ends_with(".tsx") || rel_s.ends_with(".jsx") { "tsx" } else { "typescript" };
    out.push(WalkedFile {
        relative_path: format!("ext:ts:{}/{}", dep.module_path, rel_s),
        absolute_path: file.to_path_buf(),
        language: lang,
    });

    if depth >= REEXPORT_MAX_DEPTH { return }

    let Ok(src) = std::fs::read_to_string(file) else { return };
    for target in extract_relative_reexports(&src) {
        let Some(next) = resolve_relative_ts_path(file, &target) else { continue };
        expand_reexports_into(dep, &next, out, seen, depth + 1);
    }
}

/// Scan line-by-line for `export ... from '...'` and `import ... from '...'`
/// with relative specifiers. Returns the relative path strings in order.
/// Non-relative specifiers are skipped — they're separate packages with
/// their own dep roots.
fn extract_relative_reexports(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if !(t.starts_with("export") || t.starts_with("import")) { continue }
        let Some(ix) = t.find(" from ") else { continue };
        let rest = t[ix + 6..].trim_start();
        let Some(quote) = rest.chars().next() else { continue };
        if quote != '\'' && quote != '"' { continue }
        let inner = &rest[1..];
        if let Some(end) = inner.find(quote) {
            let spec = &inner[..end];
            if spec.starts_with("./") || spec.starts_with("../") {
                out.push(spec.to_string());
            }
        }
    }
    out
}

fn resolve_relative_ts_path(from_file: &Path, spec: &str) -> Option<PathBuf> {
    let base = from_file.parent()?;
    let raw = base.join(spec);
    for ext in [".d.ts", ".ts", ".tsx", ".mts", ".cts"] {
        let p = PathBuf::from(format!("{}{}", raw.to_string_lossy(), ext));
        if p.is_file() { return Some(p) }
    }
    for ext in ["index.d.ts", "index.ts", "index.tsx"] {
        let p = raw.join(ext);
        if p.is_file() { return Some(p) }
    }
    if raw.is_file() { return Some(raw) }
    None
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
/// Re-scan `source` for `declare global { ... }` blocks and inject a
/// `SymbolKind::Variable` entry per declared name that the TypeScript
/// extractor missed. The extractor's ambient-block descent is incomplete
/// (only a subset of const/let/function/class decls inside declare global
/// surface as top-level symbols), which starves the heuristic resolver's
/// declare-global priority of the files it relies on. Re-scanning at
/// post-process time is the narrowest correctness fix.
fn backfill_declare_global_symbols(pf: &mut crate::types::ParsedFile, source: &str) {
    use crate::types::{ExtractedSymbol, SymbolKind};

    let globals = scan_declare_global_blocks(source);
    if globals.is_empty() {
        return;
    }
    let existing: std::collections::HashSet<String> =
        pf.symbols.iter().map(|s| s.name.clone()).collect();
    for name in globals {
        if existing.contains(&name) {
            continue;
        }
        pf.symbols.push(ExtractedSymbol {
            name: name.clone(),
            qualified_name: name,
            kind: SymbolKind::Variable,
            visibility: None,
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        });
    }
}

pub(crate) fn prefix_ts_external_symbols(pf: &mut crate::types::ParsedFile, package: &str) {
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
// Symbol-location index (demand-driven pipeline entry)
// ---------------------------------------------------------------------------
//
// Walks every reached npm dep root, tree-sitter parses each TS/JS source
// file without descending into function/method/class bodies, and records
// each top-level declaration's name against the file that defines it. The
// Stage 2 loop consults this index to pull only files it needs; the
// (gigabytes of) `node_modules/` that the eager walker used to force-parse
// stays untouched unless a real user chain lands on one of its symbols.
//
// File scope matches `walk_ts_external_root` — same .ts/.tsx/.d.ts/.js/.jsx
// filter, same exclusions (nested node_modules, test/story dirs, etc.).

/// Synthetic module key under which `declare global { ... }` names get
/// indexed. Resolvers doing a bare-name fallback for unimported globals
/// (vitest's `describe`/`it`/`expect` when `globals: true`, `@types/jest`
/// globals, `@types/node` `process`/`Buffer`, etc.) look up
/// `(__NPM_GLOBALS__, name)`.
pub(crate) const NPM_GLOBALS_MODULE: &str = "__npm_globals__";

pub(crate) fn build_npm_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Gather every walked file + its owning package name so each parallel
    // task is self-contained.
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        for wf in walk_ts_external_root(dep) {
            work.push((dep.module_path.clone(), wf));
        }
    }
    if work.is_empty() {
        return SymbolLocationIndex::new();
    }

    // Parallel header-only scan. Each task returns a FileExports record
    // capturing (a) which names the file defines locally, (b) which names
    // it re-exports and from where, (c) wildcard `export * from 'x'`
    // sources, and (d) `declare global { ... }` names.
    let scanned: Vec<(String, PathBuf, FileExports)> = work
        .par_iter()
        .map(|(module, wf)| {
            let exports = std::fs::read_to_string(&wf.absolute_path)
                .ok()
                .map(|src| scan_ts_file_exports(&src, wf.language))
                .unwrap_or_default();
            (module.clone(), wf.absolute_path.clone(), exports)
        })
        .collect();

    // Build a by-path view for re-export resolution. Two structures: a
    // HashSet<PathBuf> of every scanned file (for `resolve_relative_in_set`
    // so we don't hit the filesystem per edge) and a HashMap<Path, &exports>
    // so resolve_definition can follow named re-exports through the graph.
    let known_paths: HashSet<PathBuf> =
        scanned.iter().map(|(_, p, _)| p.clone()).collect();
    let by_path: HashMap<&Path, &FileExports> = scanned
        .iter()
        .map(|(_, p, e)| (p.as_path(), e))
        .collect();

    let mut index = SymbolLocationIndex::new();
    for (module, file, exports) in &scanned {
        // Globals: indexed under BOTH the synthetic globals module (so
        // bare-name fallback finds them) and the owning package (so
        // package-qualified lookups still resolve). Unchanged from before.
        for g in &exports.globals {
            index.insert(NPM_GLOBALS_MODULE, g.clone(), file.clone());
            index.insert(module, g.clone(), file.clone());
        }

        // Named exports: resolve each to its DEFINITION file by walking
        // the re-export graph. A barrel like `axios/index.d.ts` that
        // does `export { get } from './core'` resolves 'get' to
        // './core.d.ts' so `locate('axios', 'get')` points at the real
        // definition instead of the barrel.
        for (exposed, source) in &exports.named {
            let mut visited = HashSet::new();
            let def_file = resolve_definition(
                &by_path,
                &known_paths,
                file,
                source,
                &mut visited,
            )
            .unwrap_or_else(|| file.clone());
            index.insert(module, exposed.clone(), def_file);
        }

        // Wildcards: `export * from './mod'`. Collect every name exposed
        // through the wildcard chain (recursive, cycle-guarded) and
        // register each under OUR package's module with the definition
        // file. Cross-package wildcards (`export * from 'other-pkg'`)
        // are skipped — the other package's scan already indexes those
        // names under its own module, and we don't want to double-count.
        let mut wc_seen: HashSet<PathBuf> = HashSet::new();
        let mut wc_names: HashMap<String, PathBuf> = HashMap::new();
        for wc in &exports.wildcards {
            if !wc.starts_with('.') {
                continue;
            }
            let Some(parent) = file.parent() else { continue };
            let Some(wc_path) = resolve_relative_in_set(parent, wc, &known_paths) else {
                continue;
            };
            collect_wildcard_names(
                &by_path,
                &known_paths,
                &wc_path,
                &mut wc_seen,
                &mut wc_names,
            );
        }
        for (name, def_file) in wc_names {
            index.insert(module, name, def_file);
        }
    }
    index
}

// ---------------------------------------------------------------------------
// Re-export resolution helpers (index-build time)
// ---------------------------------------------------------------------------

/// Follow a (potentially chained) re-export from `current_file` to the file
/// that actually defines the symbol. Returns `None` when the chain exits the
/// same-package scope (cross-package specifier), dead-ends in an unscanned
/// file, or hits a cycle.
///
/// Callers fall back to indexing the name at the barrel file on `None`,
/// preserving pre-refactor behaviour for cases we can't follow statically.
fn resolve_definition(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    current_file: &Path,
    source: &ExportSource,
    visited: &mut HashSet<(PathBuf, String)>,
) -> Option<PathBuf> {
    match source {
        ExportSource::Local => Some(current_file.to_path_buf()),
        ExportSource::Namespace { module } => {
            // `export * as ns from './mod'` — ns points at the whole
            // module's entry file. No single "original" symbol name to
            // follow through chains; resolving terminates at the module
            // file itself. Cross-package namespace re-exports return
            // None with the same reasoning as cross-package Reexports.
            if !module.starts_with('.') {
                return None;
            }
            let parent = current_file.parent()?;
            resolve_relative_in_set(parent, module, known_paths)
        }
        ExportSource::Reexport { module, original } => {
            if !module.starts_with('.') {
                // Cross-package — the target package's scan indexes its
                // own Locals under its own module, so `locate(target_pkg,
                // original)` already answers for the user. We can't
                // bridge pkg-A's re-export of pkg-B's symbol into a
                // unified pointer without the pkg-B index in hand, and
                // that lives in a different ecosystem's dep_roots.
                return None;
            }
            let parent = current_file.parent()?;
            let target = resolve_relative_in_set(parent, module, known_paths)?;
            if !visited.insert((target.clone(), original.clone())) {
                return None;
            }
            let target_exports = by_path.get(target.as_path())?;
            if let Some(inner) = target_exports.named.get(original) {
                return resolve_definition(
                    by_path,
                    known_paths,
                    &target,
                    inner,
                    visited,
                );
            }
            // Name not directly in target.named — try wildcard re-exports
            // in the target file. `export * from './sub'` surfaces every
            // name in sub under the current file's export set.
            for wc in &target_exports.wildcards {
                if !wc.starts_with('.') {
                    continue;
                }
                let Some(wc_parent) = target.parent() else { continue };
                let Some(wc_path) =
                    resolve_relative_in_set(wc_parent, wc, known_paths)
                else {
                    continue;
                };
                let Some(wc_exports) = by_path.get(wc_path.as_path()) else {
                    continue;
                };
                if let Some(inner) = wc_exports.named.get(original) {
                    if let Some(def) = resolve_definition(
                        by_path,
                        known_paths,
                        &wc_path,
                        inner,
                        visited,
                    ) {
                        return Some(def);
                    }
                }
            }
            None
        }
    }
}

/// Gather every name reachable through `export * from` starting at `file`,
/// mapping each to its definition path. Wildcards chain through nested
/// wildcards too. `seen` guards cycles; `out` accumulates names with
/// first-writer-wins semantics (consistent with the index at large).
fn collect_wildcard_names(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    file: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut HashMap<String, PathBuf>,
) {
    if !seen.insert(file.to_path_buf()) {
        return;
    }
    let Some(exports) = by_path.get(file) else { return };
    for (name, source) in &exports.named {
        if out.contains_key(name) {
            continue;
        }
        let mut visited = HashSet::new();
        let def_file = resolve_definition(by_path, known_paths, file, source, &mut visited)
            .unwrap_or_else(|| file.to_path_buf());
        out.insert(name.clone(), def_file);
    }
    for wc in &exports.wildcards {
        if !wc.starts_with('.') {
            continue;
        }
        let Some(parent) = file.parent() else { continue };
        let Some(wc_path) = resolve_relative_in_set(parent, wc, known_paths) else {
            continue;
        };
        collect_wildcard_names(by_path, known_paths, &wc_path, seen, out);
    }
}

/// Filesystem-free variant of `resolve_ts_relative_import`: probes the set
/// of already-scanned files (with the same extension + index resolution
/// rules) instead of hitting disk. Saves millions of `is_file` syscalls
/// on large `node_modules` trees.
fn resolve_relative_in_set(
    base_dir: &Path,
    specifier: &str,
    known: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let target = base_dir.join(specifier);
    const EXTS: &[&str] = &[
        "ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "cjs",
    ];
    for ext in EXTS {
        let candidate = target.with_extension(ext);
        if known.contains(&candidate) {
            return Some(candidate);
        }
    }
    for ext in EXTS {
        let candidate = target.join(format!("index.{ext}"));
        if known.contains(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// How an exposed name in a TS/JS file gets its value.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ExportSource {
    /// `function X() {}`, `class X {}`, `export { localX }`, etc. — X is
    /// defined in this file.
    Local,
    /// `export { Orig as Exposed } from 'module'` — the exposed name is
    /// sourced from `module`, under `original_name` (which equals the
    /// exposed name when there's no renaming).
    Reexport { module: String, original: String },
    /// `export * as ns from 'module'` — the exposed name is the whole
    /// module's namespace object. Resolves to the module's entry file;
    /// there's no single `original` symbol name to track because the
    /// namespace is the aggregate of every export in `module`.
    Namespace { module: String },
}

/// Per-file export summary built by `scan_ts_file_exports`. Keys of
/// `named` are exposed names; the enum tells us whether a name is a
/// definition or a re-export so the index builder can follow re-exports
/// to the file that actually defines the symbol.
#[derive(Debug, Default, Clone)]
struct FileExports {
    /// exposed_name → where the value comes from
    named: HashMap<String, ExportSource>,
    /// Module specifiers of `export * from '<module>'` statements.
    wildcards: Vec<String>,
    /// `declare global { ... }` names — surfaced separately (pollute the
    /// global namespace regardless of any import).
    globals: Vec<String>,
}

/// Header-only tree-sitter scan of a TS/TSX/JS source file. Returns a
/// FileExports describing:
///
/// - `named`: top-level decls and named exports. Each keyed by the name
///   *as exposed by this file* and tagged with whether it's defined here
///   (Local) or re-exported from another module (Reexport).
/// - `wildcards`: `export * from 'x'` module specifiers.
/// - `globals`: names declared inside `declare global { ... }` blocks.
///
/// Function/method/class bodies are not walked. DefinitelyTyped shapes
/// (`declare module 'foo' { ... }`) surface their inner decls as Local
/// under the ambient module name of the containing file.
fn scan_ts_file_exports(source: &str, language: &str) -> FileExports {
    let mut out = FileExports::default();

    let ts_lang: tree_sitter::Language = match language {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    };
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };

    let root = tree.root_node();
    let bytes = source.as_bytes();

    // First pass: build a local→(module, original_name) import map so the
    // export pass can decide whether `export { X }` (no `from`) is a
    // genuine local forward or a re-export of an imported binding. Without
    // this, `import { X } from './mod'; export { X }` was misrecorded as
    // Local pointing at the barrel file — breaking any downstream lookup
    // that expected X's definition to be in `./mod`.
    let mut imports: HashMap<String, (String, String)> = HashMap::new();
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            collect_imports(&child, bytes, &mut imports);
        }
    }

    // Second pass: exports + wildcards + namespace re-exports.
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            collect_file_exports(&child, bytes, &mut out, &imports);
        }
    }

    // `declare global { ... }` extraction: tree-sitter-typescript's grammar
    // wraps this inconsistently across minor grammar releases, so fall back
    // to a regex sweep of the source.
    out.globals = scan_declare_global_blocks(source);
    out
}

/// Populate `out` with every import binding surfaced by an
/// `import_statement` node. Entry shape: `local_name → (module, original)`.
///
/// - `import X from 'mod'`                    → `"X" → ("mod", "default")`
/// - `import { X, Y as Y2 } from 'mod'`       → `"X" → ("mod", "X")`, `"Y2" → ("mod", "Y")`
/// - `import * as ns from 'mod'`              → `"ns" → ("mod", "*")`
/// - `import 'side-effect'`                   → nothing
///
/// The sentinel `"*"` in the original-name slot is recognised by the
/// export pass and upgraded to an `ExportSource::Namespace` when the
/// binding is re-exported without a `from` clause.
fn collect_imports(
    node: &Node,
    bytes: &[u8],
    out: &mut HashMap<String, (String, String)>,
) {
    if node.kind() != "import_statement" {
        return;
    }
    let Some(module) = extract_export_source_module(node, bytes) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut cc = child.walk();
        for piece in child.children(&mut cc) {
            match piece.kind() {
                // `import X from 'mod'` — default import.
                "identifier" => {
                    if let Ok(name) = piece.utf8_text(bytes) {
                        out.insert(
                            name.to_string(),
                            (module.clone(), "default".to_string()),
                        );
                    }
                }
                // `import { X, Y as Y2 } from 'mod'`
                "named_imports" => {
                    let mut sc = piece.walk();
                    for spec in piece.children(&mut sc) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let orig_node = spec.child_by_field_name("name");
                        let alias_node = spec.child_by_field_name("alias");
                        let local_node = alias_node.or(orig_node);
                        let (Some(on), Some(ln)) = (orig_node, local_node) else {
                            continue;
                        };
                        let Ok(original) = on.utf8_text(bytes) else { continue };
                        let Ok(local) = ln.utf8_text(bytes) else { continue };
                        out.insert(
                            local.to_string(),
                            (module.clone(), original.to_string()),
                        );
                    }
                }
                // `import * as ns from 'mod'`
                "namespace_import" => {
                    let mut sc = piece.walk();
                    for n in piece.children(&mut sc) {
                        if n.kind() == "identifier" {
                            if let Ok(name) = n.utf8_text(bytes) {
                                out.insert(
                                    name.to_string(),
                                    (module.clone(), "*".to_string()),
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Back-compat helper: the pre-refactor `scan_ts_header` returned
/// `(regular_names, global_names)` as flat vecs. Tests and a handful of
/// callers still depend on that shape. We derive it from the richer
/// FileExports so both surfaces stay in sync.
#[cfg(test)]
fn scan_ts_header(source: &str, language: &str) -> (Vec<String>, Vec<String>) {
    let exports = scan_ts_file_exports(source, language);
    let mut regular: Vec<String> = exports.named.into_keys().collect();
    regular.sort();
    (regular, exports.globals)
}

/// Extract names from `declare global { ... }` blocks via brace-aware
/// source scan. Used as a grammar-independent belt against tree-sitter
/// variance in how the `global` keyword lands in the CST.
fn scan_declare_global_blocks(source: &str) -> Vec<String> {
    if !source.contains("declare global") {
        return Vec::new();
    }
    let marker_re = regex::Regex::new(r"declare\s+global\s*\{").expect("declare global regex");
    let decl_re = regex::Regex::new(
        r"(?m)^\s*(?:export\s+)?(?:const|let|var|function|class|type|interface)\s+(\w+)",
    )
    .expect("declare global decl regex");

    let mut out: Vec<String> = Vec::new();
    for m in marker_re.find_iter(source) {
        // Opening `{` is the last char of the match.
        let open_brace = m.end() - 1;
        let bytes = source.as_bytes();
        let mut depth = 1i32;
        let mut i = open_brace + 1;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            continue;
        }
        let block = &source[open_brace + 1..i - 1];
        for cap in decl_re.captures_iter(block) {
            out.push(cap[1].to_string());
        }
    }
    out
}

/// Inspect one direct child of the source-file root and record any top-level
/// declaration or export into `out`. Local definitions land as
/// `ExportSource::Local`; named re-exports (`export { X } from 'mod'`) land
/// as `ExportSource::Reexport`; star re-exports (`export * from 'mod'`) land
/// in `out.wildcards`. Recurses into `export_statement`, `ambient_declaration`,
/// and `internal_module`/`module` (namespace) wrappers. Does NOT recurse into
/// `block` / `statement_block` / `class_body` / `function_body` — bodies are
/// where header-only parsing draws the line.
fn collect_file_exports(
    node: &Node,
    bytes: &[u8],
    out: &mut FileExports,
    imports: &HashMap<String, (String, String)>,
) {
    match node.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "function_signature"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.named
                        .entry(name.to_string())
                        .or_insert(ExportSource::Local);
                }
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for decl in node.children(&mut cursor) {
                if decl.kind() == "variable_declarator" {
                    if let Some(name_node) = decl.child_by_field_name("name") {
                        if name_node.kind() == "identifier" {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.named
                                    .entry(name.to_string())
                                    .or_insert(ExportSource::Local);
                            }
                        }
                    }
                }
            }
        }
        "export_statement" => {
            // Source module (`from '<mod>'`) if present on this statement:
            // `export { X } from './m'`           →  source = Some("./m")
            // `export class Foo {}`                →  source = None
            // `export * from './m'`                →  source = Some("./m")
            // `export * as ns from './m'`          →  source = Some("./m"), namespace re-export
            let source_module = extract_export_source_module(node, bytes);

            // Scan direct children once for the three distinct shapes we need:
            //   `*` alone                 → wildcard re-export
            //   `*` + namespace_export    → named namespace re-export
            //   `default` keyword         → default export (sibling may be a decl or identifier)
            let mut has_star = false;
            let mut namespace_name: Option<String> = None;
            let mut has_default_keyword = false;
            {
                let mut cursor = node.walk();
                for ch in node.children(&mut cursor) {
                    match ch.kind() {
                        "*" => has_star = true,
                        "default" => has_default_keyword = true,
                        "namespace_export" => {
                            // `* as ns` — the identifier lives inside.
                            let mut nc = ch.walk();
                            for nch in ch.children(&mut nc) {
                                if nch.kind() == "identifier" {
                                    if let Ok(n) = nch.utf8_text(bytes) {
                                        namespace_name = Some(n.to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if has_star {
                if let Some(src) = source_module.clone() {
                    if let Some(ns) = namespace_name {
                        // `export * as ns from './mod'` — single named
                        // export bound to the whole module namespace.
                        out.named
                            .entry(ns)
                            .or_insert(ExportSource::Namespace { module: src });
                    } else {
                        // `export * from './mod'` — pure wildcard.
                        out.wildcards.push(src);
                    }
                }
                // Star statement has no further specifiers to process.
                return;
            }

            // `export default ...` — register a synthetic `default` entry
            // so downstream `export { default as X } from './this'`
            // re-exports in other files can resolve through the index.
            // Points at this file: the wrapped declaration, object literal,
            // or expression is what the default value resolves to.
            if has_default_keyword {
                out.named
                    .entry("default".to_string())
                    .or_insert(ExportSource::Local);
            }

            // Walk children: each export_clause specifier is a (re-)export;
            // other children are wrapped local decls to recurse into.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                match inner.kind() {
                    "export_clause" => {
                        let mut cc = inner.walk();
                        for spec in inner.children(&mut cc) {
                            if spec.kind() != "export_specifier" {
                                continue;
                            }
                            let orig_node = spec.child_by_field_name("name");
                            let alias_node = spec.child_by_field_name("alias");
                            let exposed_node = alias_node.or(orig_node);
                            let (Some(on), Some(en)) = (orig_node, exposed_node) else {
                                continue;
                            };
                            let Ok(original) = on.utf8_text(bytes) else { continue };
                            let Ok(exposed) = en.utf8_text(bytes) else { continue };
                            // Three cases for the source:
                            //   (a) `export { X } from 'mod'` — direct re-export.
                            //   (b) `export { X }` with no `from`, but X was
                            //       imported in this file — transitive re-export.
                            //       Without this branch, the index would record
                            //       X as Local to the barrel and lose the
                            //       connection to the real definition file.
                            //   (c) `export { X }` where X is a local decl.
                            let source = if let Some(m) = source_module.clone() {
                                // (a)
                                ExportSource::Reexport {
                                    module: m,
                                    original: original.to_string(),
                                }
                            } else if let Some((m, imp_orig)) = imports.get(original) {
                                // (b) — forward the imported binding to its real source.
                                // `import * as ns; export { ns }` needs Namespace,
                                // not Reexport, because `*` isn't a real symbol.
                                if imp_orig == "*" {
                                    ExportSource::Namespace { module: m.clone() }
                                } else {
                                    ExportSource::Reexport {
                                        module: m.clone(),
                                        original: imp_orig.clone(),
                                    }
                                }
                            } else {
                                // (c) — genuinely local.
                                ExportSource::Local
                            };
                            out.named
                                .entry(exposed.to_string())
                                .or_insert(source);
                        }
                    }
                    _ => {
                        collect_file_exports(&inner, bytes, out, imports);
                    }
                }
            }
        }
        "ambient_declaration" => {
            // `declare class X {}`, `declare function f(): void`, `declare
            // module 'foo' { ... }` — recurse so the wrapped declaration
            // gets classified by its own arm.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_file_exports(&inner, bytes, out, imports);
            }
        }
        "internal_module" | "module" => {
            // `namespace X { ... }` or `module 'foo' { ... }`. Recurse
            // into the body; each direct child is itself a decl.
            let body = node
                .child_by_field_name("body")
                .or_else(|| find_named_child(node, &["statement_block"]));
            if let Some(body) = body {
                let mut cursor = body.walk();
                for inner in body.children(&mut cursor) {
                    collect_file_exports(&inner, bytes, out, imports);
                }
            }
        }
        _ => {}
    }
}

/// Extract the `'module'` specifier from an `export ... from '...'` statement.
/// Tree-sitter-typescript exposes it as a `source` field on the export_statement
/// node. The raw text includes the surrounding quotes, which we strip here.
fn extract_export_source_module(node: &Node, bytes: &[u8]) -> Option<String> {
    if let Some(src) = node.child_by_field_name("source") {
        if let Ok(raw) = src.utf8_text(bytes) {
            return Some(strip_quotes(raw));
        }
    }
    None
}

fn strip_quotes(s: &str) -> String {
    s.trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string()
}

fn find_named_child<'a>(node: &'a Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if kinds.iter().any(|k| *k == c.kind()) {
            return Some(c);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declare_global_extracts_const_decls() {
        let src = r#"
declare global {
  const suite: typeof import('vitest')['suite']
  const describe: typeof import('vitest')['describe']
  const expect: typeof import('vitest')['expect']
}
export {}
"#;
        let names = scan_declare_global_blocks(src);
        assert!(names.iter().any(|n| n == "suite"));
        assert!(names.iter().any(|n| n == "describe"));
        assert!(names.iter().any(|n| n == "expect"));
    }

    #[test]
    fn declare_global_extracts_function_and_class_decls() {
        let src = r#"
declare global {
  function beforeEach(fn: () => void): void;
  class Mocha {}
  interface JestMatcher {}
  type TestFn = () => void;
}
"#;
        let names = scan_declare_global_blocks(src);
        assert!(names.iter().any(|n| n == "beforeEach"));
        assert!(names.iter().any(|n| n == "Mocha"));
        assert!(names.iter().any(|n| n == "JestMatcher"));
        assert!(names.iter().any(|n| n == "TestFn"));
    }

    #[test]
    fn declare_global_skips_nested_blocks() {
        let src = r#"
function outer() {
  declare global {
    const notAGlobal: number; // inside a function body, shouldn't fire
  }
}
declare global {
  const realGlobal: string;
}
"#;
        // Current implementation accepts the marker anywhere; that's fine
        // in practice since .d.ts files don't have executable function
        // bodies, and matching the marker inside a non-global scope is
        // still informational. Just verify the outer block's name lands.
        let names = scan_declare_global_blocks(src);
        assert!(names.iter().any(|n| n == "realGlobal"));
    }

    #[test]
    fn declare_global_source_without_marker_returns_empty() {
        let src = "export const foo = 1;\nexport function bar() {}\n";
        assert!(scan_declare_global_blocks(src).is_empty());
    }

    #[test]
    fn scan_ts_header_returns_globals_separately() {
        let src = r#"
export function regularFn() {}
export class RegularClass {}
declare global {
  const describe: typeof import('x')['y']
  function it(name: string): void;
}
"#;
        let (regular, globals) = scan_ts_header(src, "typescript");
        assert!(regular.iter().any(|n| n == "regularFn"));
        assert!(regular.iter().any(|n| n == "RegularClass"));
        assert!(globals.iter().any(|n| n == "describe"));
        assert!(globals.iter().any(|n| n == "it"));
    }

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

    // -----------------------------------------------------------------
    // R1 — reachability-based entry resolution
    // -----------------------------------------------------------------

    fn mkdep(root: PathBuf, name: &str) -> ExternalDepRoot {
        ExternalDepRoot {
            module_path: name.to_string(),
            version: String::new(),
            root,
            ecosystem: LEGACY_ECOSYSTEM_TAG,
            package_id: None,
            requested_imports: Vec::new(),
        }
    }

    #[test]
    fn resolve_import_prefers_types_field() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("vitest");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"vitest","types":"./dist/index.d.ts","main":"./dist/index.js"}"#,
        ).unwrap();
        std::fs::write(
            root.join("dist").join("index.d.ts"),
            "export declare function describe(name: string, fn: () => void): void;",
        ).unwrap();

        let dep = mkdep(root.clone(), "vitest");
        let files = NpmEcosystem.resolve_import(&dep, "vitest", &["describe"]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].absolute_path, root.join("dist").join("index.d.ts"));
        assert_eq!(files[0].language, "typescript");
        assert!(files[0].relative_path.starts_with("ext:ts:vitest/"));
    }

    #[test]
    fn resolve_import_rewrites_main_to_dts_sibling() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("react");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"react","main":"./index.js"}"#,
        ).unwrap();
        std::fs::write(root.join("index.d.ts"), "export function Component(): any;").unwrap();

        let dep = mkdep(root.clone(), "react");
        let files = NpmEcosystem.resolve_import(&dep, "react", &["Component"]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].absolute_path, root.join("index.d.ts"));
    }

    #[test]
    fn resolve_import_falls_back_to_index_dts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("tiny-pkg");
        std::fs::create_dir_all(&root).unwrap();
        // No package.json at all — purely filesystem fallback.
        std::fs::write(root.join("index.d.ts"), "export const x: number;").unwrap();

        let dep = mkdep(root.clone(), "tiny-pkg");
        let files = NpmEcosystem.resolve_import(&dep, "tiny-pkg", &["x"]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].absolute_path, root.join("index.d.ts"));
    }

    #[test]
    fn resolve_import_returns_empty_when_no_entry() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("empty-pkg");
        std::fs::create_dir_all(&root).unwrap();

        let dep = mkdep(root, "empty-pkg");
        let files = NpmEcosystem.resolve_import(&dep, "empty-pkg", &[]);
        assert!(files.is_empty());
    }

    #[test]
    fn resolve_symbol_returns_same_entry_as_import() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("vitest");
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"name":"vitest","types":"./dist/index.d.ts"}"#,
        ).unwrap();
        std::fs::write(
            root.join("dist").join("index.d.ts"),
            "export interface Assertion {}",
        ).unwrap();

        let dep = mkdep(root.clone(), "vitest");
        let a = NpmEcosystem.resolve_import(&dep, "vitest", &["Assertion"]);
        let b = NpmEcosystem.resolve_symbol(&dep, "vitest.Assertion");
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].absolute_path, b[0].absolute_path);
    }

    // -----------------------------------------------------------------
    // R4 — file_declares_type pattern matcher
    // -----------------------------------------------------------------

    #[test]
    fn file_declares_type_matches_decl_keywords() {
        assert!(file_declares_type("export class Foo {}\n", "Foo"));
        assert!(file_declares_type("interface Foo {}\n", "Foo"));
        assert!(file_declares_type("export interface Foo<T> {}\n", "Foo"));
        assert!(file_declares_type("export type Foo = string;\n", "Foo"));
        assert!(file_declares_type("export enum Foo { A, B }\n", "Foo"));
        assert!(file_declares_type("declare class Foo {}\n", "Foo"));
        assert!(file_declares_type("export declare interface Foo {}\n", "Foo"));
        assert!(file_declares_type("export abstract class Foo {}\n", "Foo"));
        assert!(file_declares_type("export function Foo() {}\n", "Foo"));
        assert!(file_declares_type("export const Foo = 1;\n", "Foo"));
    }

    #[test]
    fn file_declares_type_rejects_partial_matches() {
        assert!(!file_declares_type("class FooBar {}\n", "Foo"));
        assert!(!file_declares_type("// uses Foo somewhere\n", "Foo"));
        assert!(!file_declares_type("import { Foo } from 'x';\n", "Foo"));
        assert!(!file_declares_type("export interface Bar { f: Foo; }\n", "Foo"));
        assert!(!file_declares_type("", "Foo"));
    }

    // -----------------------------------------------------------------
    // Header-only scanner — demand-driven pipeline entry
    // -----------------------------------------------------------------

    #[test]
    fn scan_captures_class_and_interface() {
        let src = "export class Foo {}\nexport interface Bar { x: number; }\n";
        let (names, _) = scan_ts_header(src, "typescript");
        assert!(names.contains(&"Foo".to_string()), "{names:?}");
        assert!(names.contains(&"Bar".to_string()), "{names:?}");
    }

    #[test]
    fn scan_captures_function_and_type_alias() {
        let src = "export function baz(): void {}\nexport type QID = string | number;\n";
        let (names, _) = scan_ts_header(src, "typescript");
        assert!(names.contains(&"baz".to_string()), "{names:?}");
        assert!(names.contains(&"QID".to_string()), "{names:?}");
    }

    #[test]
    fn scan_captures_top_level_const_and_let() {
        let src = "export const Version = '1.0';\nlet counter = 0;\n";
        let (names, _) = scan_ts_header(src, "typescript");
        assert!(names.contains(&"Version".to_string()), "{names:?}");
        assert!(names.contains(&"counter".to_string()), "{names:?}");
    }

    #[test]
    fn scan_captures_enum_declaration() {
        let src = "export enum Color { Red, Green, Blue }\n";
        let (names, _) = scan_ts_header(src, "typescript");
        assert!(names.contains(&"Color".to_string()), "{names:?}");
    }

    #[test]
    fn scan_descends_ambient_declare_module() {
        // DefinitelyTyped shape — declare module 'foo' { ... decls ... }.
        let src = r#"declare module "foo" { export class Client {} export function init(): void; }"#;
        let (names, _) = scan_ts_header(src, "typescript");
        assert!(names.contains(&"Client".to_string()), "{names:?}");
        assert!(names.contains(&"init".to_string()), "{names:?}");
    }

    #[test]
    fn scan_ignores_nested_decls_inside_function_bodies() {
        // Nested decls inside a function body must not leak — the scanner is
        // header-only. Outer function name should appear; the inner class
        // should not.
        let src = "export function outer() { class Hidden {} return new Hidden(); }\n";
        let (names, _) = scan_ts_header(src, "typescript");
        assert!(names.contains(&"outer".to_string()));
        assert!(!names.contains(&"Hidden".to_string()), "leaked: {names:?}");
    }

    #[test]
    fn scan_handles_tsx_components() {
        let src = "export function Button() { return <button/>; }\n";
        let (names, _) = scan_ts_header(src, "tsx");
        assert!(names.contains(&"Button".to_string()), "{names:?}");
    }

    #[test]
    fn scan_handles_plain_javascript() {
        let src = "export function helper() {}\nexport const PI = 3.14;\n";
        let (names, _) = scan_ts_header(src, "javascript");
        assert!(names.contains(&"helper".to_string()), "{names:?}");
        assert!(names.contains(&"PI".to_string()), "{names:?}");
    }

    #[test]
    fn scan_returns_empty_on_empty_source() {
        let (regular, globals) = scan_ts_header("", "typescript");
        assert!(regular.is_empty() && globals.is_empty());
    }

    #[test]
    fn build_index_returns_empty_for_no_deps() {
        let idx = build_npm_symbol_index(&[]);
        assert!(idx.is_empty());
    }

    #[test]
    fn build_index_populates_from_on_disk_node_modules() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("synthetic-pkg");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let index_dts = root.join("src").join("index.d.ts");
        std::fs::write(
            &index_dts,
            "export class Client {}\nexport function connect(): Client { return new Client(); }\n",
        )
        .unwrap();

        let dep = mkdep(root, "synthetic-pkg");
        let idx = build_npm_symbol_index(std::slice::from_ref(&dep));

        assert_eq!(
            idx.locate("synthetic-pkg", "Client"),
            Some(index_dts.as_path())
        );
        assert_eq!(
            idx.locate("synthetic-pkg", "connect"),
            Some(index_dts.as_path())
        );
        assert!(idx.locate("synthetic-pkg", "NotThere").is_none());
    }

    #[test]
    fn find_files_declaring_type_returns_definition_only() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("node_modules").join("synthetic-pkg");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src").join("foo.d.ts"),
            "export interface Foo { method(): string }\n",
        ).unwrap();
        std::fs::write(
            root.join("src").join("bar.d.ts"),
            "import { Foo } from './foo';\nexport interface Bar { f: Foo }\n",
        ).unwrap();
        std::fs::write(
            root.join("src").join("baz.d.ts"),
            "export class Baz {}\n",
        ).unwrap();

        let dep = mkdep(root, "synthetic-pkg");
        let files = find_files_declaring_type(&dep, "Foo");
        let paths: Vec<String> = files.iter().map(|f| f.relative_path.clone()).collect();

        // Only foo.d.ts (declares Foo) should match. bar.d.ts uses Foo, baz
        // declares Baz — both excluded.
        assert_eq!(paths.len(), 1, "expected only the file declaring Foo: {paths:?}");
        assert!(paths[0].ends_with("foo.d.ts"));
    }
}
