// =============================================================================
// languages/typescript/aliases.rs — module / alias / re-export resolution
//
// Helpers that answer "what does this import specifier actually resolve to?"
// for TypeScript and JavaScript. Pulls together tsconfig path aliases,
// DefinitelyTyped (`@types/*`) qname rewriting, npm workspace package
// lookup, deep-import sub-path extraction, the cross-package re-export
// chain walker, and the npm-package-match predicate used to classify a
// specifier as part of an installed dep.
// =============================================================================

use tracing::debug;

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{Resolution, SymbolInfo, SymbolLookup};

use super::predicates;

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Classify a consumer ref as external when its import specifier
/// rewrites through a tsconfig alias to a workspace file that only
/// re-exports the target from a bare (external) specifier.
///
/// Common pattern: `apps/x/src/i18n/client/trans.tsx` containing exactly
/// `export { Trans } from "react-i18next"` — the consumer's
/// `@/i18n/client/trans` import is effectively a re-export of the
/// Produce DefinitelyTyped qname prefixes for a bare specifier.
///
/// When a user imports `react` (runtime package with no inline types) the
/// actual type symbols live in `@types/react/*.d.ts` and are indexed under
/// the qname prefix `@types/react.*`. The TS resolver normally looks up
/// `react.createContext` which misses; this helper yields the alternate
/// `@types/`-prefixed candidates to retry.
///
/// Scoped convention: `@scope/pkg` → `@types/scope__pkg` (DefinitelyTyped's
/// escape for the inner `@`). Also yields `@types/pkg` for unscoped names.
pub(super) fn definitely_typed_qname_prefixes(specifier: &str) -> Vec<String> {
    if specifier.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(rest) = specifier.strip_prefix('@') {
        if let Some(slash) = rest.find('/') {
            let scope = &rest[..slash];
            let pkg = &rest[slash + 1..];
            out.push(format!("@types/{scope}__{pkg}"));
        }
    } else {
        out.push(format!("@types/{specifier}"));
    }
    out
}

/// external `react-i18next` package, not an internal symbol.
///
/// Returns the bare external namespace (e.g. `"react-i18next"`) when the
/// chain holds; `None` otherwise.
pub(super) fn classify_passthrough_alias(
    spec: &str,
    target: &str,
    package_id: Option<i64>,
    _project_ctx: Option<&ProjectContext>,
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    // Need the rewritten bare path. Skip when no alias matches.
    let rewritten = lookup.resolve_tsconfig_alias(package_id, spec)?;

    // Try to locate the resolved file in the index. Walk the same shape
    // resolve_via_alias uses so we land on the actual indexed file.
    const EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];
    let mut candidate_paths: Vec<String> = vec![rewritten.clone()];
    for ext in EXTS {
        candidate_paths.push(format!("{rewritten}{ext}"));
        candidate_paths.push(format!("{rewritten}/index{ext}"));
    }

    for candidate in &candidate_paths {
        let reexports = lookup.reexports_from(candidate);
        if reexports.is_empty() {
            continue;
        }
        // Look for a re-export entry that matches `target` (or a wildcard)
        // and points at a bare specifier. That bare spec is the external
        // namespace this ref resolves through.
        for (exported_name, source_module) in reexports {
            if !predicates::is_bare_specifier(source_module) {
                continue;
            }
            if exported_name != target && exported_name != "*" {
                continue;
            }
            return Some(source_module.clone());
        }
    }
    None
}

/// Resolve an import after rewriting the specifier through tsconfig
/// `paths` aliases. The rewritten value is a bare project-relative path
/// stem (e.g. `src/utils`), matched via `in_file` against both exact paths
/// and common TS extensions.
pub(super) fn resolve_via_alias(
    rewritten: &str,
    target: &str,
    edge_kind: crate::types::EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    // Try the rewritten path directly first — this catches
    // `module_to_file` hits populated by the TS ecosystem resolver.
    for sym in lookup.in_file(rewritten) {
        if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "ts_tsconfig_alias",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
    }

    // Common TS/JS file extensions — the rewritten stem usually omits them.
    const EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];

    // Two-pass strategy: first try direct own-symbol matches across every
    // candidate file shape (bare + ext + /index+ext). Only if none of those
    // hit, follow re-export chains — barrel files (`export { X } from './y'`)
    // and single-line re-exports (`export { X } from 'pkg'`) never carry
    // own symbols, so an in_file miss doesn't mean the symbol isn't there.
    let candidates: Vec<String> = {
        let mut v = vec![rewritten.to_string()];
        for ext in EXTS {
            v.push(format!("{rewritten}{ext}"));
            v.push(format!("{rewritten}/index{ext}"));
        }
        v
    };

    for candidate in &candidates {
        for sym in lookup.in_file(candidate) {
            if sym.name == *target && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_tsconfig_alias",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    // Own-symbol miss — walk re-export chains from every plausible path
    // shape. Follows `export { X } from './y'` and `export * from './z'`
    // up to the existing 5-hop depth limit.
    for candidate in &candidates {
        if let Some(res) = follow_reexports(candidate, target, edge_kind, lookup, 0) {
            return Some(res);
        }
    }

    // Single-default-export component files (.vue, .astro, .svelte): the
    // importer chooses its own local binding name, so `sym.name == target`
    // never matches the file-stem class symbol. Accept the single class
    // symbol as the default export.
    const DEFAULT_EXPORT_EXTS: &[&str] = &[".vue", ".astro", ".svelte"];
    if DEFAULT_EXPORT_EXTS.iter().any(|ext| rewritten.ends_with(ext)) {
        for sym in lookup.in_file(rewritten) {
            if sym.kind == "class" && predicates::kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "ts_sfc_default_import",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    None
}

/// Resolve an import that targets a sibling workspace package.
///
/// When `module_specifier` matches a package's `declared_name` (exact or by
/// prefix for deep imports like `@myorg/utils/sub/mod`), scope the symbol
/// lookup to that package and return a confidence-1.0 resolution.
///
/// For deep imports we prefer a symbol whose file path contains the import's
/// sub-path, falling back to the first kind-compatible same-name symbol in
/// the package. That keeps multi-file workspace packages resolving correctly
/// without needing to map each sub-path to an exact file.
pub(super) fn resolve_workspace_package(
    module_specifier: &str,
    target: &str,
    edge_kind: crate::types::EdgeKind,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let pkg_id = lookup.workspace_package_id(module_specifier)?;

    // If this was a deep import, compute the sub-path so we can prefer a
    // matching file. For an exact match the sub-path is empty.
    let sub_path = sub_path_for_deep_import(module_specifier, lookup);

    let syms = lookup.symbols_in_package(pkg_id);
    let mut fallback: Option<&SymbolInfo> = None;
    for sym in syms {
        if sym.name != target {
            continue;
        }
        if !predicates::kind_compatible(edge_kind, &sym.kind) {
            continue;
        }
        if let Some(sub) = &sub_path {
            if sym.file_path.contains(sub.as_str()) {
                debug!(
                    strategy = "ts_workspace_pkg",
                    module = %module_specifier,
                    target = %target,
                    sub = %sub,
                    "resolved (deep import)"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_workspace_pkg",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if fallback.is_none() {
            fallback = Some(sym);
        }
    }

    fallback.map(|sym| {
        debug!(
            strategy = "ts_workspace_pkg",
            module = %module_specifier,
            target = %target,
            "resolved"
        );
        Resolution {
            target_symbol_id: sym.id,
            confidence: 1.0,
            strategy: "ts_workspace_pkg",
            resolved_yield_type: None,
            flow_emit: None,
        }
    })
}

/// Compute the sub-path portion of a deep workspace import.
///
/// Finds the longest declared_name prefix (exact match) and returns the
/// remainder after the boundary `/`. Returns `None` when `specifier` is
/// itself the declared_name (no deep path) or when no workspace package
/// matches.
pub(super) fn sub_path_for_deep_import(specifier: &str, lookup: &dyn SymbolLookup) -> Option<String> {
    if lookup.is_workspace_declared_name(specifier) {
        return None;
    }
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if lookup.is_workspace_declared_name(path) {
            return Some(specifier[path.len() + 1..].to_string());
        }
    }
    None
}

/// Follow re-export chains through barrel files.
///
/// When `in_file(module_path)` returns no match for `target_name`, this
/// function checks whether `module_path` is a barrel file that re-exports
/// the symbol from another module — and recurses until the definition is
/// found or the depth limit is reached.
///
/// Handles:
///   `export { X } from './y'`   — named re-export; follow to `./y`
///   `export { X as Z } from './y'` — aliased; the stored `target_name` is the
///                                    *original* name (before `as`), matching
///                                    what we're looking for in the source file
///   `export * from './y'`       — wildcard; try `target_name` in every
///                                 wildcard source module
pub(super) fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: crate::types::EdgeKind,
    lookup: &dyn SymbolLookup,
    depth: u32,
) -> Option<Resolution> {
    const MAX_DEPTH: u32 = 5;
    if depth >= MAX_DEPTH {
        return None;
    }

    let reexports = lookup.reexports_from(module_path);
    if reexports.is_empty() {
        return None;
    }

    // Collect wildcard sources separately — they are tried only when no named
    // re-export matched, to avoid false positives from `export * from`.
    let mut wildcard_sources: Vec<&str> = Vec::new();

    for (exported_name, source_module) in reexports {
        if predicates::is_bare_specifier(source_module) {
            continue;
        }

        if exported_name == "*" {
            wildcard_sources.push(source_module.as_str());
            continue;
        }

        if exported_name != target_name {
            continue;
        }

        // Named match: look up `target_name` in `source_module` from the
        // CONTAINING barrel's perspective. `./quick-create-button` inside
        // `apps/web/.../index.ts` resolves to a different file than the
        // same spec from elsewhere — per-source resolution is mandatory.
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && predicates::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_reexport_chain",
                    via = %module_path,
                    source = %source_module,
                    target = %target_name,
                    depth = depth,
                    "resolved via re-export"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "ts_reexport_chain",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Not directly in `source_module` — recurse (it may itself be a barrel).
        // Prefer the resolved file path so the next hop's reexports_from
        // lookup hits its file-keyed map directly.
        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) = follow_reexports(next_path, target_name, edge_kind, lookup, depth + 1) {
            return Some(res);
        }
    }

    // No named match. Try wildcard sources in order.
    for source_module in wildcard_sources {
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && predicates::kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "ts_reexport_star",
                    via = %module_path,
                    source = %source_module,
                    target = %target_name,
                    depth = depth,
                    "resolved via export-star"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "ts_reexport_star",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }

        // Recurse into wildcard sources too — chase via resolved file path.
        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) = follow_reexports(next_path, target_name, edge_kind, lookup, depth + 1) {
            return Some(res);
        }
    }

    None
}

/// Check whether a bare module specifier is an external npm package or Node.js built-in,
/// using the project manifest (package.json) directly.
///
/// M2: scoped to the source file's `package_id` when available so a package
/// that doesn't declare a dep in its own package.json doesn't inherit it
/// from a sibling workspace package.
pub(crate) fn is_manifest_ts_package(
    ctx: &ProjectContext,
    package_id: Option<i64>,
    specifier: &str,
) -> bool {
    if specifier.starts_with("node:") {
        return true;
    }
    if let Some(m) = ctx.manifests_for(package_id).get(&ManifestKind::Npm) {
        let deps = &m.dependencies;
        if deps.contains(specifier) {
            return true;
        }
        let mut path = specifier;
        while let Some(slash) = path.rfind('/') {
            path = &path[..slash];
            if deps.contains(path) {
                return true;
            }
        }
        return false;
    }
    false
}

/// Check whether a bare module specifier matches any npm package in the manifest.
///
/// Handles exact matches and deep import paths:
///   `"react"` → matches `"react"` in dependencies.
///   `"@tanstack/react-query"` → matches `"@tanstack/react-query"`.
///   `"react-dom/client"` → matches `"react-dom"` after stripping the subpath.
pub(super) fn is_npm_package_match(
    specifier: &str,
    deps: &std::collections::HashSet<String>,
) -> bool {
    if deps.contains(specifier) {
        return true;
    }
    // Deep import path: strip trailing subpath segments until a match is found.
    let mut path = specifier;
    while let Some(slash) = path.rfind('/') {
        path = &path[..slash];
        if deps.contains(path) {
            return true;
        }
    }
    false
}
