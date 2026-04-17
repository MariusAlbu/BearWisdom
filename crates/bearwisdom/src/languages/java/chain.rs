// =============================================================================
// java/chain.rs — Java chain-aware resolution
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use super::builtins::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `this.repo.findById()` with chain `[this, repo, findById]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "com.example.OrderService")
/// 2. `repo` → look up "com.example.OrderService.repo" field → declared_type = "OrderRepo"
/// 3. `findById` → look up "com.example.OrderRepo.findById" → resolved!
pub(super) fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
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

            // Is it a known class/type? (static access: `ClassName.method()`)
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "interface" | "enum" | "type_alias"
                )
            });
            if is_type {
                Some(name.clone())
            } else {
                // Is it a field on the enclosing class?
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

    // Phase 2: Walk intermediate segments, following field types or return types.
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

        // Try via import namespaces: {namespace}.{current_type}.{field}
        let mut found = false;
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let qualified_member = format!("{module}.{member_qname}");
                    if let Some(next_type) = lookup.field_type_name(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                    if let Some(next_type) = lookup.return_type_name(&qualified_member) {
                        current_type = next_type.to_string();
                        found = true;
                        break;
                    }
                }
            }
        }
        if found {
            continue;
        }

        return None;
    }

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];
    let candidate = format!("{current_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "java_chain_resolution",
            });
        }
    }

    // Try via wildcard imports: {namespace}.{resolved_type}.{method}
    for import in &file_ctx.imports {
        if import.is_wildcard {
            if let Some(module) = &import.module_path {
                let ns_candidate = format!("{module}.{candidate}");
                if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                    if kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "java_chain_resolution",
                        });
                    }
                }
            }
        }
    }

    // Members scoped to the resolved type.
    for sym in lookup.members_of(&current_type) {
        if sym.name == last.name && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "java_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/interface from the scope chain.
///
/// Java scope_chain: `["com.example.OrderService.create", "com.example.OrderService", "com.example"]`
/// We want "com.example.OrderService".
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface" | "enum") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: second-to-last is often the class.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
