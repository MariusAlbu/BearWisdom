// =============================================================================
// csharp/chain.rs — C# chain-aware resolution
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolLookup};
use crate::indexer::resolve::type_env::TypeEnvironment;
use super::predicates::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `this.repo.FindOne()` with chain `[this, repo, FindOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "MyNs.CatalogService")
/// 2. `repo` → look up "MyNs.CatalogService.repo" field → declared_type = "CatalogRepo"
/// 3. `FindOne` → look up "MyNs.CatalogRepo.FindOne" in the symbol index → resolved!
///
/// Also tries `{using_namespace}.{type}.{method}` for types resolved via using directives.
/// Generic substitution (e.g., `IRepository<User>`) is handled by a `TypeEnvironment`.
pub(super) fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        // Single-segment chains (e.g., `Foo()`) are handled by the regular scope chain.
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    let mut initial_generic_args: Vec<String> = Vec::new();
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // `this` / `base` → find the enclosing class from the scope chain.
            find_enclosing_class(&ref_ctx.scope_chain, lookup)
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/type? (static access: `ClassName.Method()`)
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "delegate"
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

        // Direct field_type_name lookup.
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

        // Direct return_type_name lookup.
        if let Some(raw_return) = lookup.return_type_name(&member_qname) {
            let resolved = env.resolve(raw_return);
            env.push_scope();
            current_type = resolved;
            continue;
        }

        // Try via using directives: {namespace}.{current_type}.{field}
        let mut found = false;
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let qualified_member = format!("{module}.{member_qname}");
                    if let Some(next_type) = lookup.field_type_name(&qualified_member) {
                        let resolved_type = env.resolve(next_type);
                        env.push_scope();
                        current_type = resolved_type;
                        found = true;
                        break;
                    }
                    if let Some(raw_return) = lookup.return_type_name(&qualified_member) {
                        let resolved = env.resolve(raw_return);
                        env.push_scope();
                        current_type = resolved;
                        found = true;
                        break;
                    }
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
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "csharp_chain_resolution",
            });
        }
    }

    // Try via using directives: {namespace}.{resolved_type}.{method}
    for import in &file_ctx.imports {
        if import.is_wildcard {
            if let Some(module) = &import.module_path {
                let ns_candidate = format!("{module}.{candidate}");
                if let Some(sym) = lookup.by_qualified_name(&ns_candidate) {
                    if kind_compatible(edge_kind, &sym.kind) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "csharp_chain_resolution",
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
                strategy: "csharp_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class/struct/interface name from the scope chain.
///
/// C# scope_chain entries are namespace-qualified:
/// `["MyNs.MyClass.MyMethod", "MyNs.MyClass", "MyNs"]`
/// We want "MyNs.MyClass".
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface" | "record") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: the second-to-last scope entry is often the class
    // (last is the method, second-to-last is the class).
    if scope_chain.len() >= 2 {
        return Some(scope_chain[scope_chain.len() - 2].clone());
    }
    scope_chain.last().cloned()
}
