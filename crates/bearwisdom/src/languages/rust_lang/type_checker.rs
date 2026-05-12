// =============================================================================
// rust_lang/type_checker.rs — Rust type checker
//
// Walks `MemberChain` step-by-step using a Rust-specific algorithm. Every
// type name flows through `normalize_path` (replaces `::` with `.`). Was
// `languages/rust_lang/chain.rs`; ported per decision-2026-04-27-e75.
// =============================================================================

use super::predicates::{self, normalize_path};
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::chain::simple_yield_type;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

pub struct RustChecker;

impl TypeChecker for RustChecker {
    fn language_id(&self) -> &str {
        "rust"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        predicates::kind_compatible(edge_kind, sym_kind)
    }

    fn resolve_chain(
        &self,
        chain_ref: &MemberChain,
        edge_kind: EdgeKind,
        _file_ctx: Option<&FileContext>,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let segments = &chain_ref.segments;
        if segments.len() < 2 {
            return None;
        }

        // Phase 1: root type.
        let root_type = match segments[0].kind {
            SegmentKind::SelfRef => find_enclosing_type(&ref_ctx.scope_chain, lookup),
            SegmentKind::Identifier => {
                let name = &segments[0].name;

                if let Some(local_type) = lookup.local_type(name) {
                    Some(normalize_path(&local_type))
                } else {
                    let is_type = lookup.types_by_name(name).iter().any(|s| {
                        matches!(
                            s.kind.as_str(),
                            "struct" | "enum" | "trait" | "type_alias" | "class"
                        )
                    });
                    if is_type {
                        Some(normalize_path(name))
                    } else {
                        let mut found = None;
                        for scope in &ref_ctx.scope_chain {
                            let field_qname = format!("{scope}.{name}");
                            if let Some(type_name) = lookup.field_type_name(&field_qname) {
                                found = Some(normalize_path(type_name));
                                break;
                            }
                        }
                        found.or_else(|| {
                            segments[0].declared_type.as_ref().map(|t| normalize_path(t))
                        })
                    }
                }
            }
            _ => None,
        };

        let mut current_type = root_type?;

        // Phase 2: intermediate segments.
        for seg in &segments[1..segments.len() - 1] {
            let member_qname = format!("{current_type}.{}", seg.name);

            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                current_type = normalize_path(next_type);
                continue;
            }
            if let Some(next_type) = lookup.return_type_name(&member_qname) {
                current_type = normalize_path(next_type);
                continue;
            }

            let mut found = false;
            for sym in lookup.members_of(&current_type) {
                if sym.name != seg.name {
                    continue;
                }
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    current_type = normalize_path(ft);
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    current_type = normalize_path(rt);
                    found = true;
                    break;
                }
            }
            if found {
                continue;
            }

            lookup.record_chain_miss(ChainMiss {
                current_type: current_type.clone(),
                target_name: seg.name.clone(),
            });
            return None;
        }

        // Phase 3: final segment.
        let last = &segments[segments.len() - 1];
        let candidate = format!("{current_type}.{}", last.name);

        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if self.kind_compatible(edge_kind, &sym.kind) {
                tracing::debug!(
                    strategy = "rust_chain_resolution",
                    chain_len = segments.len(),
                    resolved_type = %current_type,
                    target = %last.name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "rust_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup).map(|t| normalize_path(&t)),
                    flow_emit: None,
                });
            }
        }

        for sym in lookup.members_of(&current_type) {
            if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "rust_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup).map(|t| normalize_path(&t)),
                    flow_emit: None,
                });
            }
        }

        lookup.record_chain_miss(ChainMiss {
            current_type: current_type.clone(),
            target_name: last.name.clone(),
        });
        None
    }
}

/// Find the enclosing struct/impl/trait name from the scope chain.
pub(super) fn find_enclosing_type(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "struct" | "enum" | "trait" | "class") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
