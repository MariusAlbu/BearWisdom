// =============================================================================
// php/chain.rs — PHP chain-aware resolution
// =============================================================================

use crate::indexer::resolve::chain_walker::external_type_qname;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use super::predicates::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `$this->repo->findOne()` with chain `[this, repo, findOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "App.Controllers.UserController")
/// 2. `repo` → look up "App.Controllers.UserController.repo" field → declared_type = "UserRepo"
/// 3. `findOne` → look up "App.Controllers.UserRepo.findOne" → resolved!
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

            // Is it a known class/type? (static access: `ClassName::method()`)
            // PHP traits use "class" kind in the index.
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "interface" | "enum" | "type_alias"
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

        // Try via use statement namespaces.
        let mut found = false;
        for import in &file_ctx.imports {
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
        if found {
            continue;
        }

        // External-type fallback: `current_type` may be a short name like
        // "Builder" whose external symbol lives as "laravel/framework.Builder".
        // Resolve to the full external qname and retry member lookups.
        if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
            let ext_member = format!("{ext_qname}.{}", seg.name);
            if let Some(next_type) = lookup.field_type_name(&ext_member) {
                current_type = next_type.to_string();
                continue;
            }
            if let Some(next_type) = lookup.return_type_name(&ext_member) {
                current_type = next_type.to_string();
                continue;
            }
            // External type known but member not typed — keep walking
            // with the full external qname so Phase 3 can still match.
            current_type = ext_qname;
            continue;
        }

        return None;
    }

    // Phase 3: Resolve the final segment.
    let last = &segments[segments.len() - 1];

    // Resolve `current_type` to its external qname if needed
    // (e.g., "Builder" -> "laravel/framework.Builder").
    let effective_type = external_type_qname(&current_type, lookup)
        .unwrap_or_else(|| current_type.clone());

    let candidate = format!("{effective_type}.{}", last.name);

    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "php_chain_resolution",
            });
        }
    }

    // Try via use-statement namespaces.
    for import in &file_ctx.imports {
        if let Some(module) = &import.module_path {
            let ns_candidate = format!("{module}.{candidate}");
            if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "php_chain_resolution",
                    });
                }
            }
        }
    }

    for sym in lookup.members_of(&effective_type) {
        if sym.name == last.name && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.90,
                strategy: "php_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/interface from the scope chain.
/// PHP traits use SymbolKind::Class in the index.
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
