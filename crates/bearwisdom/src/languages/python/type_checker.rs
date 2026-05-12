// =============================================================================
// python/type_checker.rs — Python type checker
//
// Walks `MemberChain` step-by-step using a Python-specific algorithm.
// Was `languages/python/chain.rs` (free function `resolve_via_chain`);
// ported onto the `TypeChecker` trait per decision-2026-04-27-e75.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::chain::simple_yield_type;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

pub struct PythonChecker;

impl TypeChecker for PythonChecker {
    fn language_id(&self) -> &str {
        "python"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        predicates::kind_compatible(edge_kind, sym_kind)
    }

    /// Walk a MemberChain step-by-step following field/return types.
    ///
    /// For `self.db.query()` with chain `[self, db, query]`:
    /// 1. `self` → enclosing class from scope_chain
    /// 2. `db`   → look up "ClassName.db" field → "DatabaseSession"
    /// 3. `query` → look up "DatabaseSession.query" → resolved
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

        // Phase 1: Determine the root type.
        let root_type = match segments[0].kind {
            SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
            SegmentKind::Identifier => {
                let name = &segments[0].name;

                if let Some(local_type) = lookup.local_type(name) {
                    Some(local_type)
                } else {
                    let is_type = lookup.types_by_name(name).iter().any(|s| {
                        matches!(
                            s.kind.as_str(),
                            "class" | "struct" | "interface" | "enum" | "type_alias"
                        )
                    });
                    if is_type {
                        Some(name.clone())
                    } else {
                        let mut found = None;
                        for scope in &ref_ctx.scope_chain {
                            let field_qname = format!("{scope}.{name}");
                            if let Some(type_name) = lookup.field_type_name(&field_qname) {
                                found = Some(type_name.to_string());
                                break;
                            }
                        }
                        found.or_else(|| segments[0].declared_type.clone())
                    }
                }
            }
            _ => None,
        };

        let mut current_type = root_type?;

        // Phase 2: Walk intermediate segments.
        for seg in &segments[1..segments.len() - 1] {
            let member_qname = format!("{current_type}.{}", seg.name);

            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                current_type = next_type.to_string();
                continue;
            }
            if let Some(next_type) = lookup.return_type_name(&member_qname) {
                current_type = next_type.to_string();
                continue;
            }

            let mut found = false;
            for sym in lookup.members_of(&current_type) {
                if sym.name != seg.name {
                    continue;
                }
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    current_type = ft.to_string();
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    current_type = rt.to_string();
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

        // Phase 3: Final segment.
        let last = &segments[segments.len() - 1];
        let candidate = format!("{current_type}.{}", last.name);

        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if self.kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "python_chain_resolution",
                    chain_len = segments.len(),
                    resolved_type = %current_type,
                    target = %last.name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "python_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup),
                    flow_emit: None,
                });
            }
        }

        for sym in lookup.members_of(&current_type) {
            if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "python_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup),
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

/// Find the enclosing class name from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    scope_chain.last().cloned()
}
