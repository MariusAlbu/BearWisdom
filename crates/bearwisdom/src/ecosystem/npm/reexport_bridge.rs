// =============================================================================
// ecosystem/npm/reexport_bridge.rs — cross-package re-export alias resolution
//
// A package's barrel may bind a name whose declaration lives in a SIBLING
// dep root (`export { X } from 'other-pkg'` where `other-pkg` is itself a
// scanned dep root).
// The `(module, name) → file` index keeps pointing at the barrel for these:
// materialized symbols are qname-prefixed by their OWN file's package, so
// relocating the slot to the declaring file would key the declaration under
// a prefix the importing module's lookups never probe. Instead this walker
// resolves the declaration's file + declared name, and the caller records
// the pair as a qname ALIAS (`{module}.{name}` → the declaration) that the
// lookup layer consults. The declaration keeps its single identity; the
// alias only adds the extra qname.
//
// Derived entirely from the re-export specifiers in the scanned barrels and
// the dep roots' own entry maps — no package knowledge lives here.
// =============================================================================

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::npm_package_name_from_spec;
use super::symbol_index::{
    language_for_ext, resolve_pkg_relative, resolve_relative_in_set, same_package_deep_path,
};
use super::ts_scan::{scan_ts_file_exports, ExportSource, FileExports};
use super::walk::{resolve_relative_ts_path, REEXPORT_MAX_DEPTH};

/// Shared read-only state for one bridge walk: the header-scan maps covering
/// every scanned dep root, plus the per-package entry/root maps that resolve a
/// bare specifier to a file.
pub(super) struct BridgeCtx<'a> {
    pub by_path: &'a HashMap<&'a Path, &'a FileExports>,
    pub known_paths: &'a HashSet<PathBuf>,
    /// `module_path → the package's `.` entry file`.
    pub pkg_entry: &'a HashMap<String, PathBuf>,
    /// Full published-subpath specifier (`pkg/sub`) → its entry file.
    pub subpath_entry: &'a HashMap<String, PathBuf>,
    /// Bare module name → the package's root directory on disk.
    pub bare_pkg_root: &'a HashMap<String, PathBuf>,
}

/// Resolve `original`, re-exported via the bare specifier `spec` naming a
/// sibling dep root, to `(declaring_file, declared_name)`. `None` when the
/// target package is not a scanned dep root, the chain dead-ends, exceeds the
/// hop bound, or terminates in a namespace re-export (no single declaration
/// to alias).
pub(super) fn resolve_cross_package_reexport(
    ctx: &BridgeCtx<'_>,
    spec: &str,
    original: &str,
    visited: &mut HashSet<(PathBuf, String)>,
    depth: u32,
) -> Option<(PathBuf, String)> {
    if depth > REEXPORT_MAX_DEPTH {
        return None;
    }
    let bare = npm_package_name_from_spec(spec);
    let start = ctx
        .pkg_entry
        .get(spec)
        .or_else(|| ctx.subpath_entry.get(spec))
        .cloned()
        .or_else(|| {
            // A deep specifier into the target package (`pkg/dist/inner`)
            // with no published subpath entry resolves against its disk root.
            let root = ctx.bare_pkg_root.get(bare)?;
            let rel = same_package_deep_path(spec, bare)?;
            resolve_pkg_relative(root, rel)
        })?;
    let root = ctx.bare_pkg_root.get(bare).map(PathBuf::as_path);
    resolve_named(ctx, &start, original, bare, root, visited, depth)
}

/// Find `name`'s declaration starting at `file` (owned by package `pkg_name`),
/// following named re-exports, same-package deep specifiers, wildcards, and
/// further cross-package hops. Prefers the pre-scanned export maps; a file the
/// entry closure never scanned is read and header-scanned from disk.
fn resolve_named(
    ctx: &BridgeCtx<'_>,
    file: &Path,
    name: &str,
    pkg_name: &str,
    pkg_root: Option<&Path>,
    visited: &mut HashSet<(PathBuf, String)>,
    depth: u32,
) -> Option<(PathBuf, String)> {
    if depth > REEXPORT_MAX_DEPTH {
        return None;
    }
    if !visited.insert((file.to_path_buf(), name.to_string())) {
        return None;
    }
    let scanned;
    let exports: &FileExports = match ctx.by_path.get(file) {
        Some(e) => e,
        None => {
            let src = std::fs::read_to_string(file).ok()?;
            scanned = scan_ts_file_exports(&src, language_for_ext(file));
            &scanned
        }
    };
    if let Some(source) = exports.named.get(name) {
        return match source {
            ExportSource::Local => Some((file.to_path_buf(), name.to_string())),
            // A namespace binding has no single declared symbol to alias.
            ExportSource::Namespace { .. } => None,
            ExportSource::Reexport { module, original } => {
                follow_spec(ctx, file, module, original, pkg_name, pkg_root, visited, depth)
            }
        };
    }
    // Not named directly — the declaring file may be behind a wildcard.
    for wc in &exports.wildcards {
        if let Some(hit) = follow_spec(ctx, file, wc, name, pkg_name, pkg_root, visited, depth) {
            return Some(hit);
        }
    }
    None
}

/// Follow one re-export hop: a relative specifier resolves against the current
/// file's directory, a same-package deep specifier against the package root,
/// and a specifier naming another package recurses into that package's entry.
#[allow(clippy::too_many_arguments)]
fn follow_spec(
    ctx: &BridgeCtx<'_>,
    from: &Path,
    spec: &str,
    name: &str,
    pkg_name: &str,
    pkg_root: Option<&Path>,
    visited: &mut HashSet<(PathBuf, String)>,
    depth: u32,
) -> Option<(PathBuf, String)> {
    if spec.starts_with('.') {
        let target = resolve_target_file(ctx, from, spec)?;
        return resolve_named(ctx, &target, name, pkg_name, pkg_root, visited, depth + 1);
    }
    if let Some(rel) = same_package_deep_path(spec, pkg_name) {
        let target = resolve_pkg_relative(pkg_root?, rel)?;
        return resolve_named(ctx, &target, name, pkg_name, pkg_root, visited, depth + 1);
    }
    resolve_cross_package_reexport(ctx, spec, name, visited, depth + 1)
}

/// Resolve a relative specifier against `from`'s directory: the already-
/// scanned path set first (no syscalls), the filesystem second — a hop into a
/// file the entry closure never scanned.
fn resolve_target_file(ctx: &BridgeCtx<'_>, from: &Path, spec: &str) -> Option<PathBuf> {
    from.parent()
        .and_then(|dir| resolve_relative_in_set(dir, spec, ctx.known_paths))
        .or_else(|| resolve_relative_ts_path(from, spec))
}
