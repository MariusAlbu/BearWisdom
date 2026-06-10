// =============================================================================
// ecosystem/npm/symbol_index.rs — (module, name) → file index for demand resolution
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::ecosystem::externals::ExternalDepRoot;
use crate::ecosystem::SymbolLocationIndex;
use crate::walker::WalkedFile;

use super::package_declares_globals;
use super::ts_scan::{scan_declare_global_blocks, scan_ts_file_exports, ExportSource, FileExports};
use super::walk::{
    extract_relative_reexports, is_test_or_story_file, resolve_package_entry_path,
    resolve_relative_ts_path, walk_ts_dep_entry_only, walk_ts_external_root, REEXPORT_MAX_DEPTH,
};

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
    // imports. Globals-declaring packages bypass the entry restriction:
    // their `declare global { ... }` blocks anywhere in the tree need
    // to surface as global symbols, so `package_declares_globals` gates
    // a full walk instead of entry-only.
    let mut work: Vec<(String, WalkedFile)> = Vec::new();
    for dep in dep_roots {
        let walked = if package_declares_globals(&dep.root) {
            walk_ts_external_root(dep)
        } else {
            walk_ts_dep_entry_only(dep)
        };
        for wf in walked {
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

        // Named exports: resolve each to its DEFINITION file by walking
        // the re-export graph. A barrel like `axios/index.d.ts` that
        // does `export { get } from './core'` resolves 'get' to
        // './core.d.ts' so `locate('axios', 'get')` points at the real
        // definition instead of the barrel.
        for (exposed, source) in &exports.named {
            let mut visited = HashSet::new();
            let def_file = resolve_definition(&by_path, &known_paths, file, source, &mut visited)
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
            let Some(parent) = file.parent() else {
                continue;
            };
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
pub(crate) fn resolve_definition(
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
                return resolve_definition(by_path, known_paths, &target, inner, visited);
            }
            // Name not directly in target.named — try wildcard re-exports
            // in the target file. `export * from './sub'` surfaces every
            // name in sub under the current file's export set.
            for wc in &target_exports.wildcards {
                if !wc.starts_with('.') {
                    continue;
                }
                let Some(wc_parent) = target.parent() else {
                    continue;
                };
                let Some(wc_path) = resolve_relative_in_set(wc_parent, wc, known_paths) else {
                    continue;
                };
                let Some(wc_exports) = by_path.get(wc_path.as_path()) else {
                    continue;
                };
                if let Some(inner) = wc_exports.named.get(original) {
                    if let Some(def) =
                        resolve_definition(by_path, known_paths, &wc_path, inner, visited)
                    {
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
pub(crate) fn collect_wildcard_names(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    file: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut HashMap<String, PathBuf>,
) {
    if !seen.insert(file.to_path_buf()) {
        return;
    }
    let Some(exports) = by_path.get(file) else {
        return;
    };
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
        let Some(parent) = file.parent() else {
            continue;
        };
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
pub(crate) fn resolve_relative_in_set(
    base_dir: &Path,
    specifier: &str,
    known: &HashSet<PathBuf>,
) -> Option<PathBuf> {
    let target = base_dir.join(specifier);
    const EXTS: &[&str] = &["ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "cjs"];
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
