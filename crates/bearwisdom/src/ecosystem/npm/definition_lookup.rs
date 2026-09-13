// =============================================================================
// npm/definition_lookup.rs — the file that defines one exported name
//
// Follows a package's re-export chain from its entry to the declaration that
// exports `name`, on disk and inside an already-walked file set, expanding
// wildcard re-exports along the way.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::symbol_index::{language_for_ext, resolve_pkg_relative, same_package_deep_path};
use super::ts_scan::{scan_ts_file_exports, ExportSource, FileExports};
use super::walk::{resolve_relative_ts_path, REEXPORT_MAX_DEPTH};

/// Follow `original`'s definition starting from `file`, a same-package deep
/// specifier's resolved target that was never part of the initial
/// entry+reexport-closure header scan (`by_path`/`known_paths`) — so each hop
/// reads and scans the file directly instead of consulting the pre-built maps.
///
/// The deep specifier's resolved file is sometimes a directory-index barrel
/// (`export * from './sub'`), not the file that declares `original` — the
/// `next/dist/server/after` shape, where the specifier resolves to
/// `after/index.d.ts`, itself only re-exporting from sibling `./after.ts`.
/// Follows a further RELATIVE named re-export or wildcard the same way
/// `resolve_definition`'s in-memory chain does; a hop naming a genuinely
/// different package stops here (that package's own scan owns it).
fn resolve_on_disk(file: &Path, original: &str, depth: u32) -> Option<PathBuf> {
    if depth > REEXPORT_MAX_DEPTH {
        return None;
    }
    let src = std::fs::read_to_string(file).ok()?;
    let exports = scan_ts_file_exports(&src, language_for_ext(file));
    if let Some(source) = exports.named.get(original) {
        return match source {
            ExportSource::Local => Some(file.to_path_buf()),
            ExportSource::Reexport {
                module,
                original: inner,
            } if module.starts_with('.') => {
                let target = resolve_relative_ts_path(file, module)?;
                resolve_on_disk(&target, inner, depth + 1)
            }
            ExportSource::Namespace { module } if module.starts_with('.') => {
                resolve_relative_ts_path(file, module)
            }
            _ => None,
        };
    }
    for wc in &exports.wildcards {
        if !wc.starts_with('.') {
            continue;
        }
        let Some(target) = resolve_relative_ts_path(file, wc) else {
            continue;
        };
        if let Some(found) = resolve_on_disk(&target, original, depth + 1) {
            return Some(found);
        }
    }
    None
}

/// Follow a (potentially chained) re-export from `current_file` to the file
/// that actually defines the symbol. Returns `None` when the chain exits the
/// package scope (a genuinely cross-package specifier), dead-ends in an
/// unscanned file, or hits a cycle.
///
/// Callers fall back to indexing the name at the barrel file on `None`,
/// preserving pre-refactor behaviour for cases we can't follow statically.
pub(crate) fn resolve_definition(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    current_file: &Path,
    source: &ExportSource,
    pkg_name: &str,
    pkg_root: Option<&Path>,
    visited: &mut HashSet<(PathBuf, String)>,
) -> Option<PathBuf> {
    match source {
        ExportSource::Local => Some(current_file.to_path_buf()),
        ExportSource::Namespace { module } => {
            // `export * as ns from './mod'` — ns points at the whole
            // module's entry file. No single "original" symbol name to
            // follow through chains; resolving terminates at the module
            // file itself. A specifier naming a genuinely different package
            // returns None with the same reasoning as cross-package Reexports.
            if !module.starts_with('.') {
                let rel = same_package_deep_path(module, pkg_name)?;
                return resolve_pkg_relative(pkg_root?, rel);
            }
            let parent = current_file.parent()?;
            resolve_relative_in_set(parent, module, known_paths)
        }
        ExportSource::Reexport { module, original } => {
            if !module.starts_with('.') {
                // A same-package deep specifier — the file it names may not
                // be part of `known_paths` (a package-internal implementation
                // file the entry+reexport-closure scan never reached), and the
                // specifier itself may resolve to a directory-index barrel
                // that only re-exports further (`export * from './sub'`), so
                // `resolve_on_disk` follows the chain to `original`'s actual
                // declaring file rather than trusting the first hop.
                //
                // A specifier naming a genuinely DIFFERENT package declines —
                // the target package's own scan indexes its Locals under its
                // own module, so `locate(target_pkg, original)` already
                // answers for the user. We can't bridge pkg-A's re-export of
                // pkg-B's symbol into a unified pointer without pkg-B's
                // index in hand, and that lives in a different ecosystem's
                // dep_roots.
                let rel = same_package_deep_path(module, pkg_name)?;
                let start = resolve_pkg_relative(pkg_root?, rel)?;
                return resolve_on_disk(&start, original, 0);
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
                    pkg_name,
                    pkg_root,
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
                    if let Some(def) = resolve_definition(
                        by_path,
                        known_paths,
                        &wc_path,
                        inner,
                        pkg_name,
                        pkg_root,
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
pub(crate) fn collect_wildcard_names(
    by_path: &HashMap<&Path, &FileExports>,
    known_paths: &HashSet<PathBuf>,
    file: &Path,
    pkg_name: &str,
    pkg_root: Option<&Path>,
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
        let def_file = resolve_definition(
            by_path,
            known_paths,
            file,
            source,
            pkg_name,
            pkg_root,
            &mut visited,
        )
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
        collect_wildcard_names(
            by_path,
            known_paths,
            &wc_path,
            pkg_name,
            pkg_root,
            seen,
            out,
        );
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
    // `known` holds the lexically-normalized paths `resolve_relative_ts_path`
    // returned during the scan; normalize the freshly-joined target the same
    // way so a `../`-laden specifier still matches its entry.
    let target = super::lexically_normalize(&base_dir.join(specifier));
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
