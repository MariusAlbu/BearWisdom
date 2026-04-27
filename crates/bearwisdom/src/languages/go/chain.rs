// =============================================================================
// go/chain.rs — Go chain-aware resolution
// =============================================================================

use crate::indexer::resolve::engine::{ChainMiss, RefContext, Resolution, SymbolInfo, SymbolLookup};
use crate::type_checker::type_env::TypeEnvironment;
use super::predicates::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `s.repo.FindOne()` with chain `[s, repo, FindOne]`:
/// 1. `s` (Identifier) → look up as a field on the enclosing receiver type
///    (e.g., scope_chain contains "main.Server" → look for "main.Server.s")
/// 2. `repo` → look up "ResolvedType.repo" field → field_type_name = "UserRepo"
/// 3. `FindOne` → look up "UserRepo.FindOne" in the symbol index → resolved!
///
/// Go has no `this`/`self` keyword — the first segment is always an identifier.
/// Generic substitution (Go 1.18+) is handled by a `TypeEnvironment`.
pub(super) fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        // Single-segment chains are handled by the regular scope-chain strategies.
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    // In Go, the first segment is always an Identifier (receiver var, package name, or
    // local variable). No SelfRef — Go has no `this`.
    let mut initial_generic_args: Vec<String> = Vec::new();
    let root_type = match segments[0].kind {
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // R5: per-file flow inference takes precedence over global lookups.
            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
                // Is it a known type? (static/package-level access: `pkg.Func()`)
                let is_type = lookup.types_by_name(name).iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if is_type {
                    Some(name.clone())
                } else {
                    // Is it a field/variable on the enclosing receiver type?
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

    // Build a TypeEnvironment for this chain walk.
    let mut env = TypeEnvironment::new();

    if !initial_generic_args.is_empty() {
        env.enter_generic_context(&current_type, &initial_generic_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }

    // Phase 2: Walk intermediate segments, following field types or return types.
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

        // R5: call-site type args (`Do[User]()`) bind the method's own generic
        // params before its return type resolves.
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

        // Members fallback: find the segment among direct children of current_type.
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

        // Lost the chain — record a miss for R4 reload.
        lookup.record_chain_miss(ChainMiss {
            current_type: current_type.clone(),
            target_name: seg.name.clone(),
        });
        return None;
    }

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    // Direct qualified name match.
    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
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
            });
        }
    }

    // Members match, scoped to the resolved type.
    for sym in lookup.members_of(&current_type) {
        if sym.name == last.name && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "go_chain_resolution",
                resolved_yield_type: generic_yield_type(sym, &last.type_args, lookup, &mut env),
            });
        }
    }

    lookup.record_chain_miss(ChainMiss {
        current_type: current_type.clone(),
        target_name: last.name.clone(),
    });
    None
}

/// R5: compute the yield type of a resolved Phase-3 symbol, honoring
/// call-site generic arguments and the active `TypeEnvironment`.
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

/// Find the enclosing struct/interface name from the scope chain.
///
/// scope_chain for a method `(s *Server) Handle()` is
/// `["main.Server.Handle", "main.Server", "main"]` — we want `"main.Server"`.
#[allow(dead_code)]
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: the penultimate scope entry is often the receiver type.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
