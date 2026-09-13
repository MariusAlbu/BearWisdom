// =============================================================================
// ecosystem/npm/symbol_index.rs — (module, name) → file index for demand resolution
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::SymbolLocationIndex;
use crate::walker::WalkedFile;

pub(crate) use super::definition_lookup::{
    collect_wildcard_names, resolve_definition, resolve_relative_in_set,
};
use super::reexport_bridge::{resolve_cross_package_reexport, BridgeCtx};
use super::subpath_entries::resolve_package_subpath_entries;
use super::ts_scan::{scan_ts_file_exports, ExportSource, FileExports};
use super::walk::{
    expand_reexports_into, resolve_package_entry_path, resolve_relative_ts_path,
    walk_ts_dep_entry_only, walk_ts_external_root,
};
use super::{npm_package_name_from_spec, package_declares_globals, probe_global_decl_files};

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
// File scope is the package entry plus its relative-reexport closure
// (`walk_ts_dep_entry_only`); globals-declaring packages additionally
// union in the canonical globals-declaration files (`probe_global_decl_files`).

/// Synthetic module key under which `declare global { ... }` names get
/// indexed. Resolvers doing a bare-name fallback for unimported globals
/// (vitest's `describe`/`it`/`expect` when `globals: true`, `@types/jest`
/// globals, `@types/node` `process`/`Buffer`, etc.) look up
/// `(__NPM_GLOBALS__, name)`.
pub(crate) const NPM_GLOBALS_MODULE: &str = "__npm_globals__";

pub(crate) fn build_npm_symbol_index(dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
    // Entry-only header scan: resolve each dep's type-entry file (via
    // package.json `types`/`exports`/`main` ↔ `.d.ts` companion) and
    // follow its relative re-exports up to REEXPORT_MAX_DEPTH. Files
    // reachable only through deep import paths
    // (e.g. `import x from 'rxjs/operators'`) stay OUT of the symbol
    // index — `resolve_symbol`'s `find_files_declaring_type` fallback
    // pulls them on-demand when the chain walker asks for a name that
    // isn't indexed.
    //
    // Full-tree walks were the dominant cost of npm externals indexing:
    // material-ui ships ~3 K declaration files, lodash/rxjs/three.js
    // similar — almost all of them unreachable from the user's actual
    // imports. A package's globals contributions live in its entry, the
    // entry's relative-reexport closure, and a handful of canonical
    // globals files (`globals.d.ts`, `dist/globals.d.ts`, …) — never in
    // every leaf `.d.ts`. So a globals-declaring package gets the same
    // bounded entry+reexport walk as a non-globals package, unioned with
    // the `probe_global_decl_files` set. Files reachable only through
    // deep import paths stay locatable on demand via `resolve_symbol`.
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    // Each package's own published subpath entries, keyed by full specifier
    // (`pkg/sub` → its entry file). A barrel entry that does `export * from
    // 'pkg/sub'` (a self-subpath wildcard, not a relative one) needs this to
    // surface the subpath's names under the main module too.
    let mut subpath_entry: HashMap<String, PathBuf> = HashMap::new();
    // Each package's `.` entry keyed by module specifier — lets a cross-package
    // wildcard (`export * from 'other-pkg'`) surface the other package's exported
    // names under THIS module, so a bare import from a barrel package
    // (`import { computed } from 'vue'`, vue being `export * from '@vue/runtime-dom'`)
    // locates a name the re-exported package defines.
    let mut pkg_entry: HashMap<String, PathBuf> = HashMap::new();
    // Each reached package's own root directory, keyed by its bare module
    // name — lets a re-export resolve a Node self-reference specifier
    // (`next/dist/server/...` inside the `next` package's own `.d.ts` files)
    // against the package's disk root instead of declining it as cross-package.
    let bare_pkg_root: HashMap<String, PathBuf> = dep_roots
        .iter()
        .map(|d| (d.module_path.clone(), d.root.clone()))
        .collect();
    for dep in dep_roots {
        if let Some(entry) = resolve_package_entry_path(dep) {
            super::types_companion::insert_package_entry(&mut pkg_entry, &dep.module_path, entry);
        }
        let walked = if package_declares_globals(&dep.root) {
            union_entry_and_globals(dep)
        } else {
            walk_ts_dep_entry_only(dep)
        };
        for wf in walked {
            work.extend(
                super::types_companion::module_keys(&dep.module_path)
                    .into_iter()
                    .map(|m| (m, wf.clone())),
            );
        }
        // Concrete subpath exports (`preact/hooks`, `rxjs/ajax`) ship their own
        // declaration entry the package-root walk never reaches. Index each
        // under its full specifier (`module + suffix`) so a ref tagged with the
        // deep module — which import resolution preserves — locates the symbol.
        for (suffix, entry) in resolve_package_subpath_entries(dep) {
            let module = format!("{}{}", dep.module_path, suffix);
            subpath_entry.insert(module.clone(), entry.clone());
            let mut seen: HashSet<PathBuf> = HashSet::new();
            let mut walked = Vec::new();
            expand_reexports_into(dep, &entry, &mut walked, &mut seen, 0);
            for wf in walked {
                work.push((module.clone(), wf));
            }
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
    let known_paths: HashSet<PathBuf> = scanned.iter().map(|(_, p, _)| p.clone()).collect();
    let by_path: HashMap<&Path, &FileExports> =
        scanned.iter().map(|(_, p, e)| (p.as_path(), e)).collect();

    let mut index = SymbolLocationIndex::new();
    for (module, file, exports) in &scanned {
        // Globals: indexed under BOTH the synthetic globals module (so
        // bare-name fallback finds them) and the owning package (so
        // package-qualified lookups still resolve). Unchanged from before.
        for g in &exports.globals {
            index.insert(NPM_GLOBALS_MODULE, g.clone(), file.clone());
            index.insert(module, g.clone(), file.clone());
        }

        let bare_name = npm_package_name_from_spec(module);
        let pkg_root = bare_pkg_root.get(bare_name).map(PathBuf::as_path);

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
                bare_name,
                pkg_root,
                &mut visited,
            )
            .unwrap_or_else(|| {
                // A named re-export from a SIBLING dep root keeps the
                // barrel as its located file, but records the resolved
                // declaration as a qname-alias bridge so
                // `{module}.{name}` lookups reach the declaration's
                // single identity under its own package prefix.
                if let ExportSource::Reexport {
                    module: spec,
                    original,
                } = source
                {
                    if !spec.starts_with('.') && npm_package_name_from_spec(spec) != bare_name {
                        let ctx = BridgeCtx {
                            by_path: &by_path,
                            known_paths: &known_paths,
                            pkg_entry: &pkg_entry,
                            subpath_entry: &subpath_entry,
                            bare_pkg_root: &bare_pkg_root,
                        };
                        let mut bridge_visited = HashSet::new();
                        if let Some((target_file, target_name)) = resolve_cross_package_reexport(
                            &ctx,
                            spec,
                            original,
                            &mut bridge_visited,
                            0,
                        ) {
                            index.push_reexport_alias(module, exposed, target_file, target_name);
                        }
                    }
                }
                file.clone()
            });
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
            let wc_path = if wc.starts_with('.') {
                let Some(parent) = file.parent() else {
                    continue;
                };
                match resolve_relative_in_set(parent, wc, &known_paths) {
                    Some(p) => p,
                    None => continue,
                }
            } else if let Some(entry) = subpath_entry.get(wc) {
                // `export * from 'pkg/sub'` where `pkg/sub` is one of THIS
                // package's own published subpath entries (a barrel re-exporting
                // a subpath). The subpath's names are indexed under `pkg/sub`,
                // but the user imports them from `pkg` — surface them here too.
                // A cross-package wildcard is skipped (that package's own scan
                // indexes its names under its own module; the barrel binds via
                // re-export-following once its `.` entry is materialized).
                entry.clone()
            } else {
                continue;
            };
            collect_wildcard_names(
                &by_path,
                &known_paths,
                &wc_path,
                bare_name,
                pkg_root,
                &mut wc_seen,
                &mut wc_names,
            );
        }
        for (name, def_file) in wc_names {
            index.insert(module, name, def_file);
        }
    }
    super::module_registration::register_module_entries(
        &mut index,
        &pkg_entry,
        &subpath_entry,
        &scanned,
    );
    index
}

/// Files to index for a package that contributes runtime globals: the
/// entry + its relative-reexport closure (same bounded set the non-globals
/// branch uses) unioned with the canonical globals-declaration files. The
/// union keeps every `declare global { ... }` symbol locatable without
/// pulling the package's full leaf tree.
///
/// Fail open: when the entry can't be resolved, the relative-reexport
/// closure is unavailable, so fall back to the full-tree walk rather than
/// risk dropping a reexport-reachable type behind a non-canonical entry.
/// Over-pull is safe; under-pull is a resolution regression.
fn union_entry_and_globals(dep: &ExternalDepRoot) -> Vec<WalkedFile> {
    if resolve_package_entry_path(dep).is_none() {
        return walk_ts_external_root(dep);
    }
    let mut out = walk_ts_dep_entry_only(dep);
    let mut seen: HashSet<PathBuf> = out.iter().map(|wf| wf.absolute_path.clone()).collect();
    for wf in probe_global_decl_files(dep) {
        if seen.insert(wf.absolute_path.clone()) {
            out.push(wf);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Re-export resolution helpers (index-build time)
// ---------------------------------------------------------------------------

/// A re-export module specifier that names THIS SAME package by its own bare
/// name (`next/dist/server/...` inside one of the `next` package's own
/// `.d.ts` files) — Node's self-reference resolution, not a foreign package.
/// Returns the remainder path relative to the package root. `None` when the
/// specifier names a genuinely different package.
pub(super) fn same_package_deep_path<'a>(module: &'a str, pkg_name: &str) -> Option<&'a str> {
    module.strip_prefix(pkg_name)?.strip_prefix('/')
}

/// Resolve a same-package deep specifier's remainder path against the
/// package's disk root, reusing the relative-specifier resolver
/// (`resolve_relative_ts_path` joins against `from_file.parent()`, so a
/// synthetic anchor rooted at `pkg_root` makes that parent `pkg_root`
/// itself) — including its `.js` → `.d.ts` companion mapping for
/// rollup-bundled packages. Hits the filesystem directly rather than
/// `known_paths`: a package-internal implementation file is rarely part of
/// the entry+reexport-closure scan that built `known_paths`.
pub(super) fn resolve_pkg_relative(pkg_root: &Path, rel: &str) -> Option<PathBuf> {
    let anchor = pkg_root.join("__pkg_root__");
    resolve_relative_ts_path(&anchor, rel)
}

/// Grammar to scan `file` with, inferred from its extension — `scan_ts_file_exports`
/// only branches on tsx/javascript, defaulting to typescript for everything else
/// (`.ts`, `.d.ts`, `.mts`, `.cts`).
pub(super) fn language_for_ext(file: &Path) -> &'static str {
    match file.extension().and_then(|e| e.to_str()) {
        Some("tsx") => "tsx",
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript",
        _ => "typescript",
    }
}

#[cfg(test)]
#[path = "symbol_index_tests.rs"]
mod tests;
