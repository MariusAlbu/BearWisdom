// =============================================================================
// engine/rules/file_scoped_import — bare target named by a file-scoped
// import (Robot resource, Python library import, HCL dynamic library)
//
// Gated on `ctx.profile.file_scoped_imports` (default `Off`).
//
// When `On`, a bare target is resolved in two passes:
//   Pass 1 — match the target against a SYMBOL NAME in an imported file.
//   Pass 2 — alias-decoded: match the target against an import entry's
//             `imported_name`; on a hit, decode the entry's `alias` as
//             `{type}{separator}{member}` and bind the most-specific found
//             symbol: named member > named owning type > fallback_kind dispatch.
// =============================================================================

use crate::indexer::resolve::engine::support::normalize_name;
use crate::indexer::resolve::engine::{LookupRule, BinderContext, LookupResult};
use crate::indexer::resolve::engine::contract::SymbolInfo;
use crate::indexer::resolve::engine::contract::RESOLVED_CONFIDENCE;
use crate::type_checker::profile::language_profile::{AliasDecode, FileScopedImports, NameNormalization};
use crate::types::EdgeKind;

pub struct FileScopedImportRule;

impl LookupRule for FileScopedImportRule {
    fn name(&self) -> &'static str {
        "file_scoped_import"
    }

    fn apply(&self, ctx: &BinderContext) -> LookupResult {
        let FileScopedImports::On {
            wildcard_only,
            alias_decode,
        } = ctx.profile.file_scoped_imports
        else {
            return LookupResult::Pass;
        };
        let target = ctx.target();
        if target.is_empty() {
            return LookupResult::Pass;
        }
        let edge_kind = ctx.edge_kind();
        let norm = ctx.profile.name_normalization;
        let target_norm = normalize_name(norm, target);
        // Pass 1 — match the target against a symbol NAME in the imported file.
        for import in &ctx.file_ctx.imports {
            if wildcard_only && !import.is_wildcard {
                continue;
            }
            let Some(path) = import.module_path.as_deref() else {
                continue;
            };
            for sym in ctx.lookup.in_file(path) {
                if normalize_name(norm, &sym.name) == target_norm
                    && (ctx.kind)(edge_kind, &sym.kind)
                {
                    return LookupResult::Resolved(
                        ctx.resolved(sym.id, "default_file_scoped_import"),
                    );
                }
            }
        }
        // Pass 2 — match the target against an import entry's `imported_name`
        // and bind the symbol named by its decoded `alias`.
        if let Some(decode) = alias_decode {
            if let Some(res) = resolve_via_alias_decoded_import(
                ctx,
                decode,
                wildcard_only,
                norm,
                &target_norm,
                edge_kind,
            ) {
                return LookupResult::Resolved(res);
            }
        }
        LookupResult::Pass
    }
}

/// Alias-decode pass. For each scanned import entry whose `imported_name`
/// matches the target under `norm`, decode the entry's `alias` as
/// `{type}{separator}{member}` and bind:
///   - the symbol named `member` (most specific), else
///   - the symbol named `type`, else
///   - the first `fallback_kind` symbol in the file (the dispatch class).
/// An entry with no `alias` only participates through the fallback.
fn resolve_via_alias_decoded_import<'a>(
    ctx: &'a BinderContext<'a>,
    decode: AliasDecode,
    wildcard_only: bool,
    norm: NameNormalization,
    target_norm: &str,
    edge_kind: EdgeKind,
) -> Option<SymbolInfo> {
    for import in &ctx.file_ctx.imports {
        if wildcard_only && !import.is_wildcard {
            continue;
        }
        if normalize_name(norm, &import.imported_name) != target_norm {
            continue;
        }
        let Some(path) = import.module_path.as_deref() else {
            continue;
        };
        let (type_name, member_name) = match import.alias.as_deref() {
            Some(alias) => match alias.split_once(decode.separator) {
                Some((t, m)) => ((!t.is_empty()).then_some(t), (!m.is_empty()).then_some(m)),
                None => ((!alias.is_empty()).then_some(alias), None),
            },
            None => (None, None),
        };
        // Most specific: a named member.
        if let Some(member) = member_name {
            for sym in ctx.lookup.in_file(path) {
                if sym.name == member && (ctx.kind)(edge_kind, &sym.kind) {
                    return Some(SymbolInfo {
                        target_symbol_id: sym.id,
                        confidence: RESOLVED_CONFIDENCE,
                        strategy: "default_alias_decoded_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        // Next: the named owning type.
        if let Some(ty) = type_name {
            for sym in ctx.lookup.in_file(path) {
                if sym.name == ty && (ctx.kind)(edge_kind, &sym.kind) {
                    return Some(SymbolInfo {
                        target_symbol_id: sym.id,
                        confidence: RESOLVED_CONFIDENCE,
                        strategy: "default_alias_decoded_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        // Fallback: the dispatch type the file owns.
        if let Some(fallback_kind) = decode.fallback_kind {
            for sym in ctx.lookup.in_file(path) {
                if sym.kind == fallback_kind && (ctx.kind)(edge_kind, &sym.kind) {
                    return Some(SymbolInfo {
                        target_symbol_id: sym.id,
                        confidence: RESOLVED_CONFIDENCE,
                        strategy: "default_alias_decoded_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "file_scoped_import_tests.rs"]
mod tests;
