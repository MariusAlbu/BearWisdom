// =============================================================================
// engine/rules/reexport_following — follow project-internal re-export chains
//
// When a file imports a name (or wildcard-imports a module) and that module
// does not define the name but re-exports it (`export { X } from './y'`,
// `pub use crate::bar::X`, `export * from './z'`), walk the re-export chain
// to the module that actually defines the name.
//
// Only follows INTERNAL re-export hops; entries tagged `is_reexport=false` are
// never in the map. Cross-package re-exports are `reexport_chain`'s job.
// =============================================================================

use tracing::debug;

use crate::indexer::resolve::engine::contract::{SymbolInfo, SymbolLookup, RESOLVED_CONFIDENCE};
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::types::EdgeKind;

pub struct ReexportFollowingRule;

impl LookupRule for ReexportFollowingRule {
    fn name(&self) -> &'static str {
        "reexport_following"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let from_file = ctx.file_ctx.file_path.as_str();

        for import in &ctx.file_ctx.imports {
            if !import.is_wildcard
                && import.imported_name != "*"
                && import.imported_name != target
            {
                continue;
            }
            let Some(module) = import.module_path.as_deref() else {
                continue;
            };
            if module.is_empty() {
                continue;
            }
            let resolved = match ctx.lookup.resolve_module_from(from_file, module) {
                Some(p) => p.to_string(),
                None if is_relative_specifier(module) => module.to_string(),
                None => continue,
            };
            if let Some(res) =
                follow_reexports(&resolved, target, edge_kind, ctx.kind, ctx.lookup, 0)
            {
                return LookupResult::Resolved(res);
            }
        }
        LookupResult::Pass
    }
}

/// Walk re-export chains from `module_path` to the module that defines
/// `target_name`. Mirrors `type_checker::core::reexport::follow_reexports`
/// but accepts `&dyn Fn` instead of a bare fn pointer so the rule engine's
/// kind predicate closure can be passed through without requiring a coercion.
fn follow_reexports(
    module_path: &str,
    target_name: &str,
    edge_kind: EdgeKind,
    kind_compatible: &dyn Fn(EdgeKind, &str) -> bool,
    lookup: &dyn SymbolLookup,
    depth: u32,
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
                return Some(SymbolInfo {
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

        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) =
            follow_reexports(next_path, target_name, edge_kind, kind_compatible, lookup, depth + 1)
        {
            return Some(res);
        }
    }

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
                return Some(SymbolInfo {
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

        let next = lookup
            .resolve_module_from(module_path, source_module)
            .map(|s| s.to_string());
        let next_path: &str = next.as_deref().unwrap_or(source_module);
        if let Some(res) =
            follow_reexports(next_path, target_name, edge_kind, kind_compatible, lookup, depth + 1)
        {
            return Some(res);
        }
    }

    None
}

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
    Some(SymbolInfo {
        target_symbol_id: first.id,
        confidence: RESOLVED_CONFIDENCE,
        strategy,
        resolved_yield_type: None,
        flow_emit: None,
    })
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

/// A specifier is relative — and therefore project-internal — when it starts
/// with `.`, `/`, or is a Windows drive path.
fn is_relative_specifier(s: &str) -> bool {
    s.starts_with('.') || s.starts_with('/') || (s.len() >= 2 && s.as_bytes()[1] == b':')
}

#[cfg(test)]
#[path = "reexport_following_tests.rs"]
mod tests;
