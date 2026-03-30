// =============================================================================
// go/chain.rs — Go chain-aware resolution
// =============================================================================

use super::super::super::engine::{RefContext, Resolution, SymbolLookup};
use super::builtins::kind_compatible;
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
    let root_type = match segments[0].kind {
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known type? (static/package-level access: `pkg.Func()`)
            let is_type = lookup.by_name(name).iter().any(|s| {
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

        // Try by_name fallback with namespace prefix.
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

        // Lost the chain — can't determine the next type.
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
            });
        }
    }

    // Try by name, scoped to the resolved type.
    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "go_chain_resolution",
            });
        }
    }

    None
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
