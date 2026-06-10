// =============================================================================
// type_checker/core/reexport.rs — generic re-export chain walker
//
// One language-agnostic walk that follows a name through re-exporting modules
// to the module that actually defines it:
//
//   TS   `export { X } from './y'`   ·   `export * from './z'`
//   Rust `pub use crate::bar::X`     ·   `pub use crate::bar::*`
//   Nim  `export results`
//
// The walk consults `SymbolLookup::reexports_from`, which is populated ONLY
// from refs the extractor tagged `is_reexport=true`. A private import never
// enters that map, so this walk is structurally incapable of forwarding a name
// through a module that merely imports it (Invariant #2).
// =============================================================================

use tracing::debug;

use crate::indexer::resolve::engine::{Resolution, SymbolLookup, RESOLVED_CONFIDENCE};
use crate::types::EdgeKind;

/// Follow re-export chains from `module_path` to the module that defines
/// `target_name`.
///
/// When `module_path` re-exports `target_name` from another module
/// (`export { X } from './y'`, `pub use crate::bar::X`), resolve into that
/// module — recursing until the definition is found or `MAX_DEPTH` is reached.
///
/// Handles:
///   - named re-export   — follow to the source module, match by name
///   - aliased re-export  — the stored `target_name` is the *original* name
///                          (before `as`), matching the source-file symbol
///   - wildcard re-export — try `target_name` in every `*` source module,
///                          only after no named re-export matched
///
/// `kind_compatible` decides whether a candidate symbol's kind is a plausible
/// target for `edge_kind` — passed in so the walk stays language-agnostic.
///
/// A re-export whose source module does not resolve to a known file is skipped:
/// unindexed bare packages are the externals stage's job, not this per-file
/// walk. If the source resolves to an already-indexed `ext:` file, following it
/// is still safe because only explicit re-export entries enter this map.
pub(crate) fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: fn(EdgeKind, &str) -> bool,
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

    // Wildcard sources are tried only when no named re-export matched, to avoid
    // false positives from `export * from`.
    let mut wildcard_sources: Vec<&str> = Vec::new();

    for (exported_name, source_module) in reexports {
        // Skip unresolved bare specifiers. A relative source (`./y`, `../z`) is
        // internal by construction and always followed. A non-relative source
        // (`react`, Rust `crate::bar`, Nim `results`) is followed when module
        // resolution can map it to a known file. That file may be `ext:` when
        // dependency discovery has already indexed it; unresolved bare packages
        // remain the externals stage's job.
        if !is_relative_specifier(source_module) {
            match lookup.resolve_module_from(module_path, source_module) {
                Some(_) => {}
                _ => continue,
            }
        }

        if exported_name == "*" {
            wildcard_sources.push(source_module.as_str());
            continue;
        }

        if exported_name != target_name {
            continue;
        }

        // Named match: look up `target_name` in `source_module` from the
        // CONTAINING module's perspective. A relative spec like `./button`
        // resolves to a different file depending on the containing file, so
        // per-source resolution is mandatory.
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "reexport_chain",
                    via = %module_path,
                    source = %source_module,
                    target = %target_name,
                    depth = depth,
                    "resolved via re-export"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "reexport_chain",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if !is_relative_specifier(source_module) {
            if let Some(res) = resolve_reexport_by_matching_file(
                lookup,
                source_module,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_chain",
            ) {
                return Some(res);
            }
        }

        // Not directly in `source_module` — it may itself be a re-exporting
        // module. Recurse, preferring the resolved file path so the next hop's
        // `reexports_from` lookup hits its file-keyed map directly.
        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) = follow_reexports(
            next_path,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
        ) {
            return Some(res);
        }
    }

    // No named match. Try wildcard sources in order.
    for source_module in wildcard_sources {
        for sym in lookup.in_module_from(module_path, source_module) {
            if sym.name == target_name && kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "reexport_star",
                    via = %module_path,
                    source = %source_module,
                    target = %target_name,
                    depth = depth,
                    "resolved via export-star"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: RESOLVED_CONFIDENCE,
                    strategy: "reexport_star",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        if !is_relative_specifier(source_module) {
            if let Some(res) = resolve_reexport_by_matching_file(
                lookup,
                source_module,
                target_name,
                edge_kind,
                kind_compatible,
                "reexport_star",
            ) {
                return Some(res);
            }
        }

        // Recurse into wildcard sources too — chase via the resolved file path.
        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) = follow_reexports(
            next_path,
            target_name,
            edge_kind,
            kind_compatible,
            lookup,
            depth + 1,
        ) {
            return Some(res);
        }
    }

    None
}

/// A specifier is *relative* — and therefore project-internal — when it starts
/// with `.` or `/`, or is a Windows drive path. The negation marks a "bare"
/// (npm package / crate-name) specifier, which is followed only when it
/// resolves to an internal file. Kept local so the walk carries no
/// per-language dependency.
fn is_relative_specifier(s: &str) -> bool {
    s.starts_with('.') || s.starts_with('/') || (s.len() >= 2 && s.as_bytes()[1] == b':')
}

fn resolve_reexport_by_matching_file(
    lookup: &dyn SymbolLookup,
    source_module: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: fn(EdgeKind, &str) -> bool,
    strategy: &'static str,
) -> Option<Resolution> {
    let mut matches = lookup.by_name(target_name).iter().filter(|sym| {
        sym.name == target_name
            && kind_compatible(edge_kind, &sym.kind)
            && file_path_matches_module(&sym.file_path, source_module)
    });
    let first = matches.next()?;
    let first_file = first.file_path.as_ref();
    if matches.any(|sym| sym.file_path.as_ref() != first_file) {
        return None;
    }
    Some(Resolution {
        target_symbol_id: first.id,
        confidence: RESOLVED_CONFIDENCE,
        strategy,
        resolved_yield_type: None,
        flow_emit: None,
    })
}

fn file_path_matches_module(file_path: &str, source_module: &str) -> bool {
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
#[path = "reexport_tests.rs"]
mod tests;
