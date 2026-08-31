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
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::ecosystem::manifest::npm::NpmManifest;
use crate::ecosystem::manifest::ManifestReader;
use crate::walker::WalkedFile;
use rayon::prelude::*;
use tree_sitter::{Node, Parser};

pub const ID: EcosystemId = EcosystemId::new("npm");

/// Legacy ecosystem tag persisted in `package_deps.ecosystem` and
/// `ExternalDepRoot::ecosystem`. Renamed to "npm" in Phase 4 alongside a
/// DB migration; kept here so no schema change is required in Phase 2.
pub(super) const LEGACY_ECOSYSTEM_TAG: &str = "typescript";

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
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Package
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }
    fn manifest_specs(&self) -> &'static [ManifestSpec] {
        MANIFESTS
    }

    fn workspace_package_files(&self) -> &'static [(&'static str, &'static str)] {
        &[("package.json", "npm")]
    }

    fn pruned_dir_names(&self) -> &'static [&'static str] {
        // Cache + framework build outputs that nest under user packages and
        // should never be treated as workspace package roots themselves.
        &[
            "node_modules",
            "bower_components",
            ".next",
            ".nuxt",
            ".svelte-kit",
            ".turbo",
            ".nyc_output",
        ]
    }

    fn activation(&self) -> EcosystemActivation {
        // Project deps via `package.json`. A bare directory of `.ts` /
        // `.tsx` / `.js` / `.vue` / `.svelte` / `.astro` files with no
        // manifest can't be resolved against external npm coordinates,
        // so dropping the LanguagePresent shotgun is correct per the
        // trait doc.
        EcosystemActivation::ManifestMatch
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_ts_externals(ctx.project_root)
    }

    fn walk_root(&self, dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        walk_ts_external_root(dep)
    }

    fn supports_reachability(&self) -> bool {
        true
    }

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

    fn resolve_symbol(&self, dep: &ExternalDepRoot, fqn: &str) -> Vec<WalkedFile> {
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

    fn post_process_parsed(
        &self,
        _dep: &ExternalDepRoot,
        parsed: &mut crate::types::ParsedFile,
        arena: &crate::type_checker::core::types::TypeArena,
    ) {
        ts_post_process_external(parsed, arena);
    }

    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        build_npm_symbol_index(dep_roots)
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

/// Probe whether a package's entry .d.ts contributes runtime globals.
///
/// Returns `true` when the package's main type-declaration file contains
/// an explicit `declare global { ... }` block or a top-level
/// `declare namespace ...` declaration. Both constructs add symbols to
/// the project's global ambient scope without an explicit `import`, so
/// the indexer needs to walk such packages even when no user code ever
/// names the package itself.
///
/// Catches every package whose author opted into globals via the
/// canonical TS pattern: `@angular/localize` (`$localize`), test runners
/// like `vitest` / `jest` / `mocha` / `jasmine` (`describe`, `it`,
/// `expect`, `beforeEach`), `@types/jquery` (`$`, `jQuery`),
/// `@types/google.maps` (`google.maps.*`), `@types/chrome`,
/// `@types/cypress` (`cy`, `Cypress`), and any future library that uses
/// the same construct.
///
/// Reads at most three small files: the package's `package.json`
/// `types`/`typings` entry, plus standard fallback names (`index.d.ts`,
/// `types/index.d.ts`). Bounded I/O — typically <30 KB across all
/// candidates.
pub(crate) fn package_declares_globals(pkg_root: &Path) -> bool {
    for entry in candidate_globals_entry_files(pkg_root) {
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if file_contributes_globals(&content) {
            return true;
        }
    }
    false
}

/// True when `pkg_root` contains at least one `.scss` source file within
/// the first two directory levels (excluding `node_modules`, test dirs, and
/// dot dirs). Used as a gate condition to retain SCSS mixin packages in the
/// dep-root list even when user SCSS source never writes `@use 'pkg-name'`
/// — a pattern common to SCSS test frameworks that are runner-injected.
pub(crate) fn package_ships_scss(pkg_root: &Path) -> bool {
    package_ships_scss_bounded(pkg_root, 0)
}

fn package_ships_scss_bounded(dir: &Path, depth: u32) -> bool {
    if depth >= 2 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == "node_modules"
                    || name.starts_with('.')
                    || matches!(name, "test" | "tests" | "__tests__" | "docs" | "examples")
                {
                    continue;
                }
            }
            if package_ships_scss_bounded(&path, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("scss"))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}

/// Resolve the candidate `.d.ts` files we should probe for declare-global
/// content. Reads `package.json`'s `types`/`typings` field if present;
/// otherwise probes standard entry filenames at the package root.
fn candidate_globals_entry_files(pkg_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(pkg_text) = std::fs::read_to_string(pkg_root.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&pkg_text) {
            for field in ["types", "typings"] {
                if let Some(p) = json.get(field).and_then(|v| v.as_str()) {
                    let candidate = pkg_root.join(p);
                    if candidate.is_file() {
                        out.push(candidate);
                    }
                }
            }
        }
    }
    // Some packages declare their globals ONLY in a separate `globals.d.ts`
    // (vitest `globals: true`, @vue/test-utils), never in the `types` entry —
    // so the entry-only check misses them and the probe below never fires.
    for name in [
        "index.d.ts",
        "types/index.d.ts",
        "globals.d.ts",
        "dist/globals.d.ts",
    ] {
        let candidate = pkg_root.join(name);
        if candidate.is_file() {
            out.push(candidate);
        }
    }
    out
}

/// True when the file body contains an explicit `declare global { ... }`
/// block OR a top-level `declare namespace ...` declaration. These are the
/// two TypeScript constructs that contribute symbols to the global ambient
/// scope. Cheap substring + line-prefix check — full parsing happens later
/// in `scan_declare_global_blocks` for actual extraction.
fn file_contributes_globals(content: &str) -> bool {
    if content.contains("declare global") {
        return true;
    }
    for line in content.lines() {
        let t = line.trim_start();
        if t.starts_with("declare namespace ") {
            return true;
        }
    }
    false
}


/// Try each canonical declaration-file path under `dep.root`. Return one
/// `WalkedFile` per file actually present. Probes filenames known to host
/// `declare global { ... }` blocks across `@types/jest`, `@types/mocha`,
/// `@types/node`, `vitest`, `chai`, `sinon`, etc.
pub(crate) fn probe_global_decl_files(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    const CANDIDATE_REL_PATHS: &[&str] = &[
        "globals.d.ts",
        "global.d.ts",
        "index.d.ts",
        "jest.d.ts",
        "mocha.d.ts",
        "jasmine.d.ts",
        "dist/globals.d.ts",
        "dist/global.d.ts",
        "dist/index.d.ts",
        "dist/index.d.cts",
        "dist/index.d.mts",
        "lib/index.d.ts",
        "lib/globals.d.ts",
        "types/index.d.ts",
        "types/globals.d.ts",
        // @types/jquery splits across `JQuery.d.ts`, `JQueryStatic.d.ts`,
        // `factory.d.ts`, etc. — index.d.ts is just a triple-slash hub
        // and we don't follow those refs. Probe the canonical filenames
        // directly so the actual API surface lands in the index.
        "JQuery.d.ts",
        "JQueryStatic.d.ts",
        "factory.d.ts",
        "factory-slim.d.ts",
        "misc.d.ts",
    ];
    let mut out = Vec::new();
    for rel in CANDIDATE_REL_PATHS {
        let path = dep.root.join(rel);
        if !path.is_file() {
            continue;
        }
        let virtual_path = format!("ext:ts:{}/{}", dep.module_path, rel);
        out.push(WalkedFile {
            relative_path: virtual_path,
            absolute_path: path,
            language: "typescript",
        });
    }
    out
}


pub(super) fn scan_for_scss_bounded(dir: &Path, depth: u32) -> bool {
    if depth >= 6 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(
                    name,
                    "node_modules"
                        | "target"
                        | "build"
                        | "out"
                        | "dist"
                        | ".next"
                        | ".nuxt"
                        | ".astro"
                        | ".svelte-kit"
                        | ".vite"
                        | ".turbo"
                        | ".cache"
                        | "coverage"
                ) || name.starts_with('.')
                {
                    continue;
                }
            }
            if scan_for_scss_bounded(&path, depth + 1) {
                return true;
            }
        } else if ft.is_file() {
            if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("scss"))
                .unwrap_or(false)
            {
                return true;
            }
        }
    }
    false
}


// ---------------------------------------------------------------------------
// Legacy ExternalSourceLocator impl — adapter for the indexer pipeline
// until Phase 4 migrates to Ecosystem directly.
// ---------------------------------------------------------------------------

impl ExternalSourceLocator for NpmEcosystem {
    fn ecosystem(&self) -> &'static str {
        LEGACY_ECOSYSTEM_TAG
    }

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

    fn post_process_parsed(
        &self,
        parsed: &mut crate::types::ParsedFile,
        arena: &crate::type_checker::core::types::TypeArena,
    ) {
        ts_post_process_external(parsed, arena);
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
        //    (`indexer::resolve::legacy::resolve_type_name_in_scope`)
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
// Module-path validation
// ---------------------------------------------------------------------------

/// Collapse embedded `/./` and `/../` segments and normalise backslashes in
/// a path fragment that's about to land in a virtual `ext:ts:<pkg>/<rel>`
/// URI. `resolve_relative_ts_path` joins specs like `./internal/foo` or
/// `../../foo` without normalising, so a single .d.ts can otherwise show up
/// under multiple virtual paths (`dist/types/Observable.d.ts`,
/// `dist/types/./internal/Observable.d.ts`,
/// `dist/types/internal/../Observable.d.ts`) and confuse downstream
/// dedupe + symbol prefixing.
pub(crate) fn normalize_virtual_rel(rel: &str) -> String {
    let s = rel.replace('\\', "/");
    let mut out: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|s| *s != "..") {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            _ => out.push(seg),
        }
    }
    out.join("/")
}

/// Collapse `.` and `..` components out of an absolute filesystem path
/// without touching disk (no symlink resolution, no existence check — the
/// path may name a file that doesn't exist yet at call time). Two relative
/// re-export hops that reach the same physical file by different routes
/// (`pkg/a/../b.d.ts` vs `pkg/b.d.ts`) must produce an identical `PathBuf`,
/// since every reachability-closure dedup (`HashSet<PathBuf>`) and
/// `(module, name) → PathBuf` first-writer-wins map is keyed on this
/// identity. A leading `..` that would pop past the start of the path is
/// kept literally rather than silently dropped, since there is no root
/// segment left to remove.
pub(crate) fn lexically_normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                _ => out.push(".."),
            },
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Reject `dep.module_path` shapes that would produce malformed virtual
/// paths (`ext:ts:./xxx/...`, `ext:ts:F:/xxx/...`, `ext:ts:.ignored_xxx/...`).
///
/// Every walker formats `ext:ts:{module_path}/{rel_sub}` and downstream code
/// assumes a clean npm package shape — `name` or `@scope/name`. Anything
/// else (relative specifiers, drive letters, pnpm `.ignored_*` shadows,
/// `.pnpm/` store paths, hidden dirs) breaks `ts_package_from_virtual_path`,
/// which then either returns garbage prefixes (`F:`, `.`, `.ignored_xxx`)
/// or fails to identify the package at all — leaving the chain walker
/// unable to follow library types like `Observable.pipe()` or `HTMLElement.click()`.
///
/// Reduce a possibly-deep npm specifier to just its package name. Handles
/// scoped (`@scope/pkg/sub` → `@scope/pkg`), unscoped (`pkg/sub` → `pkg`),
/// and already-bare (`pkg` → `pkg`) forms. Returns the input unchanged when
/// the layout doesn't match either shape (callers re-validate).
pub(crate) fn npm_package_name_from_spec(spec: &str) -> &str {
    if let Some(rest) = spec.strip_prefix('@') {
        // Scoped: keep the first two slash-separated segments (`@scope/name`).
        let mut iter = rest.splitn(3, '/');
        let scope = iter.next().unwrap_or("");
        let name = iter.next().unwrap_or("");
        if !scope.is_empty() && !name.is_empty() {
            let end = 1 + scope.len() + 1 + name.len(); // '@' + scope + '/' + name
            return &spec[..end];
        }
        spec
    } else {
        // Unscoped: keep the leading segment.
        match spec.find('/') {
            Some(slash) => &spec[..slash],
            None => spec,
        }
    }
}

/// Used at every `ExternalDepRoot { module_path: … }` construction site to
/// gate which paths get into the index in the first place.
pub(crate) fn is_valid_npm_module_path(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.starts_with('.') {
        return false;
    } // ./, ../, .ignored_, .pnpm
    if name.contains(':') {
        return false;
    } // F:/Work/...
    if name.contains('\\') {
        return false;
    } // windows path leak
    if name.starts_with('@') {
        // Scoped: must be exactly `@scope/name`.
        let rest = &name[1..];
        let Some((scope, pkg)) = rest.split_once('/') else {
            return false;
        };
        if scope.is_empty() || pkg.is_empty() {
            return false;
        }
        if scope.starts_with('.') || pkg.starts_with('.') {
            return false;
        }
        if pkg.contains('/') {
            return false;
        } // no nested paths under @scope
        true
    } else {
        // Unscoped: single segment, no slashes.
        !name.contains('/')
    }
}

// ---------------------------------------------------------------------------
// Node builtins — appear in package.json declared deps but have no on-disk
// source under node_modules. Skipped during walk.
// ---------------------------------------------------------------------------

fn node_builtins() -> std::collections::HashSet<&'static str> {
    [
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
        "node",
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
    ]
    .into_iter()
    .collect()
}

mod externals;
mod externals_imports;
mod externals_node_modules;
mod module_registration;
mod post_process;
mod reexport_bridge;
mod symbol_index;
mod ts_scan;
mod ts_scan_ambient;
mod walk;

pub(crate) use externals::*;
pub(crate) use externals_imports::*;
pub(crate) use externals_node_modules::*;
pub(crate) use post_process::*;
pub(crate) use symbol_index::*;
pub(crate) use ts_scan::*;
pub(crate) use walk::*;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
