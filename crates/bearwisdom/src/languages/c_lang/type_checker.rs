// =============================================================================
// c_lang/type_checker.rs — C/C++ type checker
//
// Walks `MemberChain` step-by-step using a C/C++-specific algorithm. Was
// `languages/c_lang/chain.rs`; ported per decision-2026-04-27-e75.
//
// C++ specifics:
//   - No generic substitution at this stage (template parameters are hard
//     to resolve without a full instantiation graph; covered by the
//     type_alias extraction fix for project-defined typedefs).
//   - Uses `.` as the separator in type_info keys (matching the indexer's
//     convention, even though C++ uses `::` for qualified names).
//   - SelfRef (`this->`) is handled by finding the enclosing class in the
//     scope chain.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::chain::simple_yield_type;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

pub struct CChecker;

impl TypeChecker for CChecker {
    fn language_id(&self) -> &str {
        "c"
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
                            let field_qname_cc = format!("{}.{name}", scope.replace("::", "."));
                            if let Some(type_name) = lookup.field_type_name(&field_qname_cc) {
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
        current_type = normalize_type(&current_type);

        // Phase 2: intermediate segments.
        for seg in &segments[1..segments.len() - 1] {
            let member_qname = format!("{current_type}.{}", seg.name);

            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                current_type = normalize_type(next_type);
                current_type = dereference_typedef(&current_type, lookup);
                continue;
            }

            if let Some(raw_return) = lookup.return_type_name(&member_qname) {
                current_type = normalize_type(raw_return);
                current_type = dereference_typedef(&current_type, lookup);
                continue;
            }

            let mut found = false;
            for sym in lookup.members_of(&current_type) {
                if sym.name != seg.name {
                    continue;
                }
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    current_type = normalize_type(ft);
                    current_type = dereference_typedef(&current_type, lookup);
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    current_type = normalize_type(rt);
                    current_type = dereference_typedef(&current_type, lookup);
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
        current_type = dereference_typedef(&current_type, lookup);
        let candidate = format!("{current_type}.{}", last.name);

        if let Some(sym) = lookup.by_qualified_name(&candidate) {
            if self.kind_compatible(edge_kind, &sym.kind) {
                debug!(
                    strategy = "c_chain_resolution",
                    chain_len = segments.len(),
                    resolved_type = %current_type,
                    target = %last.name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "c_chain_resolution",
                    resolved_yield_type: simple_yield_type(sym, lookup).map(|t| normalize_type(&t)),
                    flow_emit: None,
                });
            }
        }

        let matches: Vec<_> = lookup
            .members_of(&current_type)
            .iter()
            .filter(|sym| sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind))
            .cloned()
            .collect();

        match matches.len() {
            0 => {
                lookup.record_chain_miss(ChainMiss {
                    current_type: current_type.clone(),
                    target_name: last.name.clone(),
                });
                None
            }
            1 => Some(Resolution {
                target_symbol_id: matches[0].id,
                confidence: 1.0,
                strategy: "c_chain_resolution_unique",
                resolved_yield_type: simple_yield_type(&matches[0], lookup).map(|t| normalize_type(&t)),
                flow_emit: None,
            }),
            _ => Some(Resolution {
                target_symbol_id: matches[0].id,
                confidence: 0.95,
                strategy: "c_chain_resolution",
                resolved_yield_type: simple_yield_type(&matches[0], lookup).map(|t| normalize_type(&t)),
                flow_emit: None,
            }),
        }
    }
}

/// Find the enclosing class name from the scope chain.
fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct") {
                return Some(scope.clone());
            }
        }
        let normalized = scope.replace("::", ".");
        if normalized != *scope {
            if let Some(sym) = lookup.by_qualified_name(&normalized) {
                if matches!(sym.kind.as_str(), "class" | "struct") {
                    return Some(normalized);
                }
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(normalize_type(&scope_chain[scope_chain.len() - 2]));
    }
    scope_chain.last().map(|s| normalize_type(s))
}

/// Normalize a C++ qualified name for type_info lookups (`::` → `.`).
fn normalize_type(name: &str) -> String {
    name.replace("::", ".")
}

/// One-hop typedef dereference: project-defined pointer typedefs like
/// `TSocketChannelPtr` → `SocketChannel`.
fn dereference_typedef(type_name: &str, lookup: &dyn SymbolLookup) -> String {
    for sym in lookup.types_by_name(type_name) {
        if sym.kind == "type_alias" {
            if let Some(aliased) = lookup.field_type_name(&sym.qualified_name) {
                return normalize_type(aliased);
            }
        }
    }
    type_name.to_string()
}
