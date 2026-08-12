// =============================================================================
// engine/reexports_candidates — relative-source candidate generation and
// bare-source file-path matching for the re-export chain walker
//
// `follow_reexports` (engine/reexports.rs) needs two kinds of "does this file
// plausibly hold the re-export's source?" check that don't touch the graph
// walk itself: turning a relative source module into the file paths it could
// name (extension guesses, `/index` entries), and matching a bare source
// against a symbol's file path when the source isn't a resolvable specifier
// (Nim's `std/`/`pkg/` prefixes, a workspace-package barrel).
// =============================================================================

use crate::indexer::resolve::engine::contract::{SymbolInfo, SymbolLookup};
use crate::indexer::resolve::engine::reexports::reexport_resolution;
use crate::types::EdgeKind;

/// Resolve a relative re-export source to its declaring symbol by file path. The
/// source is joined against the barrel file's directory and normalized; a
/// `by_name(target_name)` candidate whose file path matches that base (with a
/// source extension or an `/index` entry appended) is the declaration. Used when
/// the per-source module-to-file map carries no mapping for the relative
/// specifier, so `in_module_from` returned nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_relative_reexport(
    lookup: &dyn SymbolLookup,
    barrel_path: &str,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    strategy: &'static str,
) -> Option<SymbolInfo> {
    let base = relative_base(barrel_path, source_module)?;
    for sym in lookup.by_name(target_name) {
        if sym.name != target_name || !kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        if relative_file_matches_base(&sym.file_path, &base) {
            return Some(reexport_resolution(sym.id, strategy));
        }
    }
    None
}

/// The candidate file paths a relative re-export source could name — the joined,
/// normalized base with each source extension and `/index` entry appended,
/// filtered to those actually indexed (i.e. that have non-empty
/// `reexports_from`, the only files `follow_reexports` can recurse into). Empty
/// when the base can't be formed.
pub(crate) fn relative_reexport_candidates(
    lookup: &dyn SymbolLookup,
    barrel_path: &str,
    source_module: &str,
) -> Vec<String> {
    let Some(base) = relative_base(barrel_path, source_module) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for cand in extension_candidates(&base) {
        if !lookup.reexports_from(&cand).is_empty() {
            out.push(cand);
        }
    }
    out
}

/// Join a relative `source_module` against `barrel_path`'s directory and collapse
/// `.`/`..` segments. Returns the extension-less base (`packages/q/src/index` +
/// `./queryClient` → `packages/q/src/queryClient`). `None` when `barrel_path` has
/// no directory portion.
pub(crate) fn relative_base(barrel_path: &str, source_module: &str) -> Option<String> {
    let normalized = barrel_path.replace('\\', "/");
    let dir = normalized.rsplit_once('/').map(|(d, _)| d)?;
    let joined = format!("{dir}/{}", source_module.replace('\\', "/"));
    let mut out: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    Some(out.join("/"))
}

/// The source-file path candidates for an extension-less `base`: the base
/// verbatim (the specifier may already carry its own extension — Dart's
/// `foo.dart` and any non-TS/JS source language), the base with each TS/JS
/// extension appended, plus the `base/index.<ext>` directory entry.
pub(crate) fn extension_candidates(base: &str) -> Vec<String> {
    const EXTS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "mts", "cts", "cjs", "svelte", "astro", "vue",
    ];
    let mut out: Vec<String> = Vec::with_capacity(EXTS.len() * 2 + 1);
    out.push(base.to_string());
    for ext in EXTS {
        out.push(format!("{base}.{ext}"));
    }
    for ext in EXTS {
        out.push(format!("{base}/index.{ext}"));
    }
    out
}

/// `true` when `file_path` is one of the source-extension / `/index` forms of
/// `base`. Both inputs are forward-slash normalized; the file matches when it
/// equals a candidate or ends with `/<candidate>` (a suffix match, so a
/// project-relative base resolves against a deeper indexed path).
pub(crate) fn relative_file_matches_base(file_path: &str, base: &str) -> bool {
    let normalized = file_path.replace('\\', "/");
    extension_candidates(base)
        .iter()
        .any(|cand| normalized == *cand || normalized.ends_with(&format!("/{cand}")))
}

/// The re-export barrels of the workspace package a bare `specifier` names —
/// the files in that package whose basename stem is one of `stems` (the
/// language's `reexport_barrel_stems`: `index` for JS/TS, `lib`/`main` for Rust)
/// and whose `reexports_from` is non-empty. Empty when `specifier` is not a
/// workspace package, `stems` is empty, or the package has no re-export barrel.
pub(crate) fn workspace_pkg_barrels(
    lookup: &dyn SymbolLookup,
    specifier: &str,
    stems: &[&str],
) -> Vec<String> {
    let Some(pkg_id) = lookup.workspace_package_id(specifier) else {
        return Vec::new();
    };
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    let mut barrels: Vec<String> = Vec::new();
    for sym in lookup.symbols_in_package(pkg_id) {
        let path = sym.file_path.as_ref();
        if !seen.insert(path.to_string()) {
            continue;
        }
        if !path_basename_stem_in(path, stems) {
            continue;
        }
        if lookup.reexports_from(path).is_empty() {
            continue;
        }
        barrels.push(path.to_string());
    }
    barrels
}

/// The symbol a bare workspace-package `specifier` DECLARES under `target_name`
/// — a sibling member that owns the type, reached as the source of a cross-member
/// re-export (`pub use member::Name`). Scans the package's own symbol set for a
/// name + edge-kind match. `None` when `specifier` is not a workspace package or
/// declares no such name.
pub(crate) fn workspace_pkg_declared_symbol(
    lookup: &dyn SymbolLookup,
    specifier: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
) -> Option<i64> {
    let pkg_id = lookup.workspace_package_id(specifier)?;
    let mut found: Option<i64> = None;
    for sym in lookup.symbols_in_package(pkg_id) {
        if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
            if found.is_some() {
                return None; // ambiguous: two same-name declarations — decline
            }
            found = Some(sym.id);
        }
    }
    found
}

/// `true` when the file's basename stem (extension dropped) is one of `stems`.
fn path_basename_stem_in(file_path: &str, stems: &[&str]) -> bool {
    let normalized = file_path.replace('\\', "/");
    let Some(basename) = normalized.rsplit('/').next() else {
        return false;
    };
    let stem = basename.split('.').next().unwrap_or(basename);
    stems.contains(&stem)
}

/// Match a re-export's bare source module to its declaring symbol by file path
/// when the source module isn't a resolvable file. Accepts only when exactly one
/// by-name candidate's file path matches the module — an ambiguous match is no
/// match. Handles Nim-style `std/`/`pkg/` prefixes and `.nim` extensions.
pub(crate) fn resolve_reexport_by_matching_file(
    lookup: &dyn SymbolLookup,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    strategy: &'static str,
) -> Option<SymbolInfo> {
    let mut matches = lookup.by_name(target_name).into_iter().filter(|sym| {
        sym.name == target_name
            && kind_compatible(edge_kind, &sym.kind)
            && reexport_file_path_matches_module(&sym.file_path, source_module)
    });
    let first = matches.next()?;
    let first_file = first.file_path.as_ref();
    if matches.any(|sym| sym.file_path.as_ref() != first_file) {
        return None;
    }
    Some(reexport_resolution(first.id, strategy))
}

/// File-path matcher for re-export module resolution. Handles Nim-style
/// `std/`/`pkg/` prefixes and `.nim` extension matching.
fn reexport_file_path_matches_module(file_path: &str, source_module: &str) -> bool {
    let trimmed = source_module.trim_matches('"').trim_matches('\'').trim();
    let stripped = trimmed
        .strip_prefix("std/")
        .or_else(|| trimmed.strip_prefix("pkg/"))
        .unwrap_or(trimmed)
        .replace('\\', "/");
    let module = stripped.trim_matches('/');
    if module.is_empty() {
        return false;
    }
    let normalized = file_path.replace('\\', "/");
    let candidates = if module.ends_with(".nim") {
        vec![module.to_string()]
    } else {
        vec![format!("{module}.nim"), format!("{module}/mod.nim")]
    };
    candidates
        .iter()
        .any(|candidate| normalized == *candidate || normalized.ends_with(&format!("/{candidate}")))
}

#[cfg(test)]
#[path = "reexports_candidates_tests.rs"]
mod tests;
