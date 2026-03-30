// =============================================================================
// ruby/chain.rs — Ruby chain-aware resolution
// =============================================================================

use super::super::super::engine::{RefContext, Resolution, SymbolLookup};
use super::builtins::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
pub(super) fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => find_enclosing_class(&ref_ctx.scope_chain, lookup),
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/namespace? (constant access: `ClassName.method`)
            // Ruby modules are stored as "namespace".
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(s.kind.as_str(), "class" | "namespace" | "interface" | "type_alias")
            });
            if is_type {
                Some(name.clone())
            } else {
                // Is it an instance variable / field on the enclosing class?
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

        // Try by_name fallback.
        let mut found = false;
        for sym in lookup.by_name(&seg.name) {
            if sym.qualified_name.starts_with(&current_type) {
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
        }
        if found {
            continue;
        }

        return None;
    }

    // Phase 3: Resolve the final segment.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "ruby_chain_resolution",
            });
        }
    }

    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "ruby_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/namespace from the scope chain.
/// Ruby modules are stored as `namespace` in the index.
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            // "namespace" covers both Ruby modules and packages.
            if matches!(sym.kind.as_str(), "class" | "namespace" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
