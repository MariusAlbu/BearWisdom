// =============================================================================
// type_checker/bare.rs — bare-name resolution for chain-less refs
//
// Engine entry for refs that don't carry a MemberChain. Mirrors the lookup
// strategies the chain walker's DefaultRootResolver uses for chain roots
// but yields a target symbol id directly so the result is a Resolution
// suitable for the resolver loop.
//
// Strategy order, all kind-filtered via the language profile:
//   1. Scope-chain walk: innermost scope first, probe `{scope}.{target}`.
//   2. Same-file: any symbol in the source file named `target`.
//   3. Fully-qualified target: `target` already contains dots — probe directly.
//   4. Import-qualified: for each import naming `target`, probe `{module}.{target}`.
//
// Language-specific behaviors (TS workspace packages, tsconfig path aliases,
// DefinitelyTyped fallback, barrel re-exports, self/this/base stripping)
// stay in the legacy resolver until per-language `LanguageEngineHooks`
// adopt them. Engine returns None when none of these strategies hits; the
// resolver loop's fallback chain still runs.
// =============================================================================

use std::str::FromStr;

use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::type_checker::profile::language_profile::{
    KindCompatibility, LanguageProfile,
};
use crate::types::{EdgeKind, SymbolKind};

/// Try to resolve a bare-name (chain-less) ref against the symbol index.
///
/// Returns `Some(Resolution)` only when a strategy matches and the kind is
/// compatible with `ref_ctx.extracted_ref.kind` per the profile's kind
/// table. Returns `None` so the resolver loop can fall back to the legacy
/// per-language resolver or the heuristic tier.
pub fn resolve_bare(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
    profile: &LanguageProfile,
) -> Option<Resolution> {
    let target = ref_ctx.extracted_ref.target_name.as_str();
    let edge_kind = ref_ctx.extracted_ref.kind;

    if edge_kind == EdgeKind::Imports {
        return None;
    }

    // 1. Scope-chain walk: innermost → outermost.
    for scope in &ref_ctx.scope_chain {
        let candidate = format!("{scope}.{target}");
        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if kind_ok(profile, edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "engine_bare_scope",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    // 2. Same-file resolution.
    for sym in lookup.in_file(&file_ctx.file_path) {
        if sym.name == target && kind_ok(profile, edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "engine_bare_same_file",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
    }

    // 3. Fully-qualified target (contains dots).
    if target.contains('.') {
        if let Some(sym) = lookup.by_qualified_name(target) {
            if kind_ok(profile, edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "engine_bare_qname",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    // 4. Import-qualified: file imports `target` from `module_path`, probe
    //    `{module_path}.{target}` for an external symbol whose qname was
    //    rewritten with the package prefix by the externals pipeline.
    for import in &file_ctx.imports {
        let matches_import = import.imported_name == target
            || import.alias.as_deref() == Some(target);
        if !matches_import {
            continue;
        }
        let Some(module_path) = import.module_path.as_deref() else {
            continue;
        };
        let candidate = format!("{module_path}.{target}");
        for sym in lookup.all_by_qualified_name(&candidate) {
            if kind_ok(profile, edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "engine_bare_import_qname",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
    }

    None
}

/// Profile-driven kind compatibility check. Unrecognised symbol kind
/// strings default to permissive so extractor typos don't silently hide
/// real symbols. Mirrors `type_checker::core::members::kind_matches`.
fn kind_ok(profile: &LanguageProfile, edge: EdgeKind, sym_kind: &str) -> bool {
    let Ok(parsed) = SymbolKind::from_str(sym_kind) else {
        return true;
    };
    KindCompatibility::check(profile.kind_compatible_table, edge, parsed)
}

#[cfg(test)]
#[path = "bare_tests.rs"]
mod tests;
