// =============================================================================
// engine/reexports — follow barrel / re-export chains to a declaring symbol
//
// `export { X } from './x'` and `export * from './x'` hops are walked from a
// module path (or a workspace-package entry) until the module that actually
// declares the target is reached; named entries win over the wildcard sweep
// and the walk is depth-bounded. Also answers the workspace-package questions
// built on the same evidence: a package's barrel files and the symbol a
// package declares under a given name.
// =============================================================================

use crate::indexer::resolve::engine::contract::{
    SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE,
};
use crate::indexer::resolve::engine::path_match::is_relative_specifier;
use crate::types::EdgeKind;

/// Walk re-export chains from `module_path` to the module that defines
/// `target_name`, returning the declaring symbol when found. A `module_path` is
/// a resolved file path (or a workspace-package entry whose `reexports_from`
/// fallback resolves it). Both `export { X } from '...'` (named) and `export *
/// from '...'` (wildcard) hops are followed; the wildcard sweep runs after the
/// named entries so a more-specific named re-export wins. `kind_compatible`
/// gates a candidate by the ref's edge kind. Bounded at `MAX_DEPTH` hops.
pub(crate) fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
    barrel_stems: &[&str],
) -> Option<SymbolInfo> {
    const MAX_DEPTH: u32 = 5;
    if depth >= MAX_DEPTH {
        return None;
    }

    let reexports = lookup.reexports_from(module_path);
    if reexports.is_empty() {
        return None;
    }

    let mut wildcard_sources: Vec<&str> = Vec::new();

    for (exported_name, source_module) in reexports {
        // A bare workspace-package source may be a consumer-scoped Cargo dependency
        // RENAME in the re-exporting file's own package (`pub use common::X` where
        // `common` = `tantivy-common`). Rewrite the alias head to the target package
        // so the hop keys on the real member; the wildcard path keeps the original.
        let renamed = if is_relative_specifier(source_module)
            || lookup.workspace_package_id(source_module).is_some()
        {
            None
        } else {
            let head = source_module.split("::").next().unwrap_or(source_module);
            lookup
                .dep_rename(lookup.package_id_for_file(module_path), head)
                .map(|t| format!("{t}{}", &source_module[head.len()..]))
        };
        let source = renamed.as_deref().unwrap_or(source_module);

        // A bare source is followable when it resolves to a file OR names a sibling
        // workspace package whose barrel can be recovered; a true external is skipped.
        if !is_relative_specifier(source)
            && lookup.resolve_module_from(module_path, source).is_none()
            && workspace_pkg_barrels(lookup, source, barrel_stems).is_empty()
            && lookup.workspace_package_id(source).is_none()
        {
            continue;
        }

        if exported_name == "*" {
            wildcard_sources.push(source_module.as_str());
            continue;
        }

        if exported_name != target_name {
            continue;
        }

        // A bare source naming a sibling workspace package may DECLARE the target
        // directly (a member that owns the type, re-exported through this module),
        // rather than forwarding it onward. Chase one hop into the package's own
        // symbol set (the cross-member re-export seam).
        if !is_relative_specifier(source) {
            if let Some(id) = workspace_pkg_declared_symbol(
                lookup,
                source,
                target_name,
                edge_kind,
                kind_compatible,
            ) {
                return Some(reexport_resolution(id, "reexport_chain"));
            }
        }

        for sym in lookup.in_module_from(module_path, source) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                return Some(reexport_resolution(sym.id, "reexport_chain"));
            }
        }
        if is_relative_specifier(source) {
            if let Some(res) = resolve_relative_reexport(
                lookup,
                module_path,
                source,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_chain",
            ) {
                return Some(res);
            }
        } else if let Some(res) = resolve_reexport_by_matching_file(
            lookup,
            source,
            target_name,
            edge_kind,
            kind_compatible,
            "reexport_chain",
        ) {
            return Some(res);
        }

        if let Some(res) = follow_reexport_source(
            module_path,
            source,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth,
            barrel_stems,
        ) {
            return Some(res);
        }
    }

    for source_module in wildcard_sources {
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                return Some(reexport_resolution(sym.id, "reexport_star"));
            }
        }
        if is_relative_specifier(source_module) {
            if let Some(res) = resolve_relative_reexport(
                lookup,
                module_path,
                source_module,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_star",
            ) {
                return Some(res);
            }
        } else if let Some(res) = resolve_reexport_by_matching_file(
            lookup,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            "reexport_star",
        ) {
            return Some(res);
        }

        if let Some(res) = follow_reexport_source(
            module_path,
            source_module,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth,
            barrel_stems,
        ) {
            return Some(res);
        }
    }

    None
}

/// Recurse `follow_reexports` into a re-export's source module. A relative or
/// resolvable source recurses on its resolved file path; a bare workspace-package
/// source recurses on each of the package's re-exporting `index` barrels (the
/// bare specifier has no `resolve_module_from` mapping, so the barrel is
/// recovered from the package's own symbol set).
#[allow(clippy::too_many_arguments)]
fn follow_reexport_source(
    module_path: &str,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
    barrel_stems: &[&str],
) -> Option<SymbolInfo> {
    if let Some(next) = lookup.resolve_module_from(module_path, source_module) {
        let next = next.to_string();
        return follow_reexports(
            &next,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
            barrel_stems,
        );
    }
    if !is_relative_specifier(source_module) {
        for barrel in workspace_pkg_barrels(lookup, source_module, barrel_stems) {
            if let Some(res) = follow_reexports(
                &barrel,
                target_name,
                edge_kind,
                kind_compatible,
                lookup,
                depth + 1,
                barrel_stems,
            ) {
                return Some(res);
            }
        }
        return None;
    }
    // A relative source with no `resolve_module_from` mapping: recurse on each
    // candidate file the joined-and-normalized path could name, so a multi-hop
    // relative re-export chain (`./a` re-exports from `./b`) still threads.
    for next in relative_reexport_candidates(lookup, module_path, source_module) {
        if let Some(res) = follow_reexports(
            &next,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
            barrel_stems,
        ) {
            return Some(res);
        }
    }
    None
}

/// Resolve a relative re-export source to its declaring symbol by file path. The
/// source is joined against the barrel file's directory and normalized; a
/// `by_name(target_name)` candidate whose file path matches that base (with a
/// source extension or an `/index` entry appended) is the declaration. Used when
/// the per-source module-to-file map carries no mapping for the relative
/// specifier, so `in_module_from` returned nothing.
#[allow(clippy::too_many_arguments)]
fn resolve_relative_reexport(
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
fn relative_base(barrel_path: &str, source_module: &str) -> Option<String> {
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

/// The source-file path candidates for an extension-less `base`: the base with
/// each source extension appended, plus the `base/index.<ext>` directory entry.
fn extension_candidates(base: &str) -> Vec<String> {
    const EXTS: &[&str] = &[
        "ts", "tsx", "js", "jsx", "mjs", "mts", "cts", "cjs", "svelte", "astro", "vue",
    ];
    let mut out: Vec<String> = Vec::with_capacity(EXTS.len() * 2);
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
fn relative_file_matches_base(file_path: &str, base: &str) -> bool {
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

/// A resolved-via-re-export `SymbolInfo` tagged with the hop strategy.
fn reexport_resolution(id: i64, strategy: &'static str) -> SymbolInfo {
    SymbolInfo {
        target_symbol_id: id,
        confidence: RESOLVED_CONFIDENCE,
        strategy,
        resolved_yield_type: None,
        flow_emit: None,
    }
}

/// Match a re-export's bare source module to its declaring symbol by file path
/// when the source module isn't a resolvable file. Accepts only when exactly one
/// by-name candidate's file path matches the module — an ambiguous match is no
/// match. Handles Nim-style `std/`/`pkg/` prefixes and `.nim` extensions.
fn resolve_reexport_by_matching_file(
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
#[path = "reexports_tests.rs"]
mod tests;
