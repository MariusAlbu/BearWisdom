// =============================================================================
// php/chain.rs — PHP chain-aware resolution
// =============================================================================

use crate::type_checker::chain::{external_type_qname, simple_yield_type};
use crate::indexer::resolve::engine::{ChainMiss, FileContext, RefContext, Resolution, SymbolLookup};
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

            // R5: per-file flow inference takes precedence over global lookups.
            if let Some(local_type) = lookup.local_type(name) {
                Some(local_type)
            } else {
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
        }
        // PHP static call: `ClassName::method()` is parsed as a `static_call_expression`
        // whose first chain segment is emitted with kind TypeAccess.  The segment name
        // IS the class name — use it directly as the root type so Phase 2/3 can walk
        // forward from `ClassName.method` (e.g. `File::whereIn()` → root = "File").
        SegmentKind::TypeAccess => {
            let name = &segments[0].name;
            // Prefer a known type symbol; fall back to the bare name so that
            // project-local classes (e.g. `App.Models.File`) which inherit from
            // an Eloquent `Model` still enter the walker and get resolved via the
            // Phase-3 inheritance walk below.
            let qualified = lookup
                .types_by_name(name)
                .iter()
                .find(|s| matches!(s.kind.as_str(), "class" | "interface" | "enum" | "type_alias"))
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| name.clone());
            Some(qualified)
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

        // R4 chain miss: upgrade short name to external qname before recording
        // so the reload pass can address the right ExternalDepRoot.
        let miss_type = external_type_qname(&current_type, lookup)
            .unwrap_or_else(|| current_type.clone());
        lookup.record_chain_miss(ChainMiss {
            current_type: miss_type,
            target_name: seg.name.clone(),
        });
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
                resolved_yield_type: simple_yield_type(sym, lookup),
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
                        resolved_yield_type: simple_yield_type(sym, lookup),
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
                resolved_yield_type: simple_yield_type(sym, lookup),
            });
        }
    }

    // Inheritance walk: when `ClassName::staticMethod()` or a chained call fails
    // on the direct class, walk parent classes via the inheritance map.
    // This covers Eloquent Model subclasses whose Builder methods are forwarded
    // via `__callStatic` — the stubs expose them on the `Model` qname.
    // Depth-bounded at 10 to guard against cycles in malformed source.
    let mut cls = effective_type.as_str();
    for _ in 0..10 {
        match lookup.parent_class_qname(cls) {
            None => break,
            Some(parent) => {
                let parent_candidate = format!("{parent}.{}", last.name);
                if let Some(sym) = lookup.by_qualified_name(&parent_candidate) {
                    if kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.85,
                            strategy: "php_chain_inherited",
                            resolved_yield_type: simple_yield_type(sym, lookup),
                        });
                    }
                }
                for sym in lookup.members_of(parent) {
                    if sym.name == last.name && kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.80,
                            strategy: "php_chain_inherited",
                            resolved_yield_type: simple_yield_type(sym, lookup),
                        });
                    }
                }
                cls = parent;
            }
        }
    }

    lookup.record_chain_miss(ChainMiss {
        current_type: effective_type,
        target_name: last.name.clone(),
    });
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
