// =============================================================================
// rust_lang/chain.rs — Rust chain-aware resolution
// =============================================================================

use super::super::super::engine::{RefContext, Resolution, SymbolLookup};
use super::builtins::{kind_compatible, normalize_path};
use crate::types::{EdgeKind, MemberChain, SegmentKind};

/// Walk a MemberChain step-by-step following field/return types.
///
/// For `self.repo.find_one()` with chain `[self, repo, find_one]`:
/// 1. `self` → find the enclosing struct/impl from scope_chain
/// 2. `repo` → look up "StructName.repo" field → field_type_name = "UserRepo"
/// 3. `find_one` → look up "UserRepo.find_one" → resolved
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

    // Phase 1: Determine the root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // `self` → find the enclosing struct/impl from scope_chain.
            find_enclosing_type(&ref_ctx.scope_chain, lookup)
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known type (static/enum access: `MyEnum::Variant`, `MyStruct::new()`)?
            let is_type = lookup.by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "struct" | "enum" | "trait" | "type_alias" | "class"
                )
            });
            if is_type {
                Some(normalize_path(name))
            } else {
                // Is it a field on the enclosing type?
                let mut found = None;
                for scope in &ref_ctx.scope_chain {
                    let field_qname = format!("{scope}.{name}");
                    if let Some(type_name) = lookup.field_type_name(&field_qname) {
                        found = Some(normalize_path(type_name));
                        break;
                    }
                }
                found.or_else(|| segments[0].declared_type.as_ref().map(|t| normalize_path(t)))
            }
        }
        _ => None,
    };

    let mut current_type = root_type?;

    // Phase 2: Walk intermediate segments.
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
        for sym in lookup.by_name(&seg.name) {
            if sym.qualified_name.starts_with(&current_type) {
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
            });
        }
    }

    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "rust_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing struct/impl/trait name from the scope chain.
/// scope_chain = ["crate.handlers.MyHandler.process", "crate.handlers.MyHandler",
///                "crate.handlers", "crate"]
/// → we want "crate.handlers.MyHandler"
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
    // Fallback: the penultimate scope is often the impl type.
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
