// =============================================================================
// go/type_checker.rs — Go type checker
//
// Walks `MemberChain` step-by-step using a Go-specific algorithm. Generics
// (Go 1.18+) handled via `TypeEnvironment`. Was `languages/go/chain.rs`;
// ported per decision-2026-04-27-e75.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    ChainMiss, FileContext, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::type_env::TypeEnvironment;
use crate::type_checker::TypeChecker;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

pub struct GoChecker;

impl TypeChecker for GoChecker {
    fn language_id(&self) -> &str {
        "go"
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

        // Phase 1: root type. Go has no `this`/`self`; first segment is always Identifier.
        let mut initial_generic_args: Vec<String> = Vec::new();
        let root_type = match segments[0].kind {
            SegmentKind::Identifier => {
                let name = &segments[0].name;

                if let Some(local_type) = lookup.local_type(name) {
                    Some(local_type)
                } else {
                    let is_type = lookup.types_by_name(name).iter().any(|s| {
                        matches!(
                            s.kind.as_str(),
                            "struct" | "interface" | "enum" | "type_alias"
                        )
                    });
                    if is_type {
                        Some(name.clone())
                    } else {
                        let mut found = None;
                        for scope in &ref_ctx.scope_chain {
                            let field_qname = format!("{scope}.{name}");
                            if let Some(type_name) = lookup.field_type_name(&field_qname) {
                                initial_generic_args = lookup
                                    .field_type_args(&field_qname)
                                    .unwrap_or(&[])
                                    .to_vec();
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
        let mut env = TypeEnvironment::new();

        if !initial_generic_args.is_empty() {
            env.enter_generic_context(&current_type, &initial_generic_args, |name| {
                lookup.generic_params(name).map(|p| p.to_vec())
            });
        }

        // Phase 2: intermediate segments.
        for seg in &segments[1..segments.len() - 1] {
            let member_qname = format!("{current_type}.{}", seg.name);

            if let Some(next_type) = lookup.field_type_name(&member_qname) {
                let new_args = lookup
                    .field_type_args(&member_qname)
                    .unwrap_or(&[])
                    .to_vec();
                let resolved_type = env.resolve(next_type);
                env.push_scope();
                if !new_args.is_empty() {
                    env.enter_generic_context(&resolved_type, &new_args, |name| {
                        lookup.generic_params(name).map(|p| p.to_vec())
                    });
                }
                current_type = resolved_type;
                continue;
            }

            if !seg.type_args.is_empty() {
                env.enter_generic_context(&member_qname, &seg.type_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }

            if let Some(raw_return) = lookup.return_type_name(&member_qname) {
                let resolved = env.resolve(raw_return);
                env.push_scope();
                current_type = resolved;
                continue;
            }

            let mut found = false;
            for sym in lookup.members_of(&current_type) {
                if sym.name != seg.name {
                    continue;
                }
                if let Some(ft) = lookup.field_type_name(&sym.qualified_name) {
                    let resolved_type = env.resolve(ft);
                    env.push_scope();
                    current_type = resolved_type;
                    found = true;
                    break;
                }
                if let Some(rt) = lookup.return_type_name(&sym.qualified_name) {
                    let resolved = env.resolve(rt);
                    env.push_scope();
                    current_type = resolved;
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
                    strategy = "go_chain_resolution",
                    chain_len = segments.len(),
                    resolved_type = %current_type,
                    target = %last.name,
                    "resolved"
                );
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "go_chain_resolution",
                    resolved_yield_type: generic_yield_type(sym, &last.type_args, lookup, &mut env),
                    flow_emit: None,
                });
            }
        }

        for sym in lookup.members_of(&current_type) {
            if sym.name == last.name && self.kind_compatible(edge_kind, &sym.kind) {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 0.95,
                    strategy: "go_chain_resolution",
                    resolved_yield_type: generic_yield_type(sym, &last.type_args, lookup, &mut env),
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

/// Yield type honoring call-site generics.
fn generic_yield_type(
    sym: &SymbolInfo,
    call_site_type_args: &[String],
    lookup: &dyn SymbolLookup,
    env: &mut TypeEnvironment,
) -> Option<String> {
    let raw = lookup
        .return_type_name(&sym.qualified_name)
        .or_else(|| lookup.field_type_name(&sym.qualified_name))?;
    if !call_site_type_args.is_empty() {
        env.enter_generic_context(&sym.qualified_name, call_site_type_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }
    Some(env.resolve(raw))
}
