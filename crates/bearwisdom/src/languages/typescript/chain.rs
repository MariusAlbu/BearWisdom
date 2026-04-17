// =============================================================================
// typescript/chain.rs — TypeScript chain-aware resolution
// =============================================================================

use crate::indexer::resolve::chain_walker::external_type_qname;
use crate::indexer::resolve::engine::{RefContext, Resolution, SymbolLookup};
use crate::indexer::resolve::type_env::TypeEnvironment;
use super::predicates::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `this.repo.findOne()` with chain `[this, repo, findOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "UserService")
/// 2. `repo` → look up "UserService.repo" field → declared_type = "UserRepo"
/// 3. `findOne` → look up "UserRepo.findOne" in the symbol index → resolved!
///
/// Generic substitution is handled by a `TypeEnvironment`: when we encounter a field
/// like `repo: Repository<User>`, we bind the Repository's type params to concrete
/// args in a new scope. When the resolved return type is a bound param (e.g., "T"),
/// `env.resolve("T")` returns the concrete type ("User").
pub(super) fn resolve_via_chain(
    chain: &MemberChain,
    edge_kind: EdgeKind,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let segments = &chain.segments;
    if segments.len() < 2 {
        // Single-segment chains (e.g., `foo()`) are handled by the regular scope chain.
        return None;
    }

    // Phase 1: Determine the root type from the first segment.
    let mut initial_generic_args: Vec<String> = Vec::new();
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // `this` → find the enclosing class from the scope chain.
            find_enclosing_class(&ref_ctx.scope_chain, lookup)
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;

            // Is it a known class/type? (static access: `ClassName.method()`)
            let is_type = lookup.types_by_name(name).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
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
                        // Capture generic args from the field declaration.
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
    // When entering a generic type (e.g., Repository<User>), we push a scope binding
    // T=User. When a return type is a bound param, env.resolve() substitutes it.
    let mut env = TypeEnvironment::new();

    // If the root was a generic field, enter its generic context now.
    if !initial_generic_args.is_empty() {
        env.enter_generic_context(&current_type, &initial_generic_args, |name| {
            lookup.generic_params(name).map(|p| p.to_vec())
        });
    }

    // Phase 2: Walk intermediate segments, following field types or return types.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        // Try field type (property access).
        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            let new_args = lookup
                .field_type_args(&member_qname)
                .unwrap_or(&[])
                .to_vec();
            // Resolve the new type through the environment (handles T → User etc).
            let resolved_type = env.resolve(next_type);
            // Transition to the new type's generic context.
            env.push_scope();
            if !new_args.is_empty() {
                env.enter_generic_context(&resolved_type, &new_args, |name| {
                    lookup.generic_params(name).map(|p| p.to_vec())
                });
            }
            current_type = resolved_type;
            continue;
        }

        // Try return type (method call result in a fluent chain).
        if let Some(raw_return) = lookup.return_type_name(&member_qname) {
            // Use TypeEnvironment to substitute type params (e.g., "T" → "User").
            let resolved = env.resolve(raw_return);
            // Clear current generic bindings and enter context for the new type.
            env.push_scope();
            current_type = resolved;
            continue;
        }

        // Members fallback scoped to the resolved type.
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

        // External-type fallback: `current_type` may be a short name like
        // "Assertion" whose external symbol lives as "chai.Assertion".
        // Resolve to the full external qname and retry member lookups.
        if let Some(ext_qname) = external_type_qname(&current_type, lookup) {
            let ext_member = format!("{ext_qname}.{}", seg.name);
            if let Some(next_type) = lookup.field_type_name(&ext_member) {
                let resolved = env.resolve(next_type);
                env.push_scope();
                current_type = resolved;
                continue;
            }
            if let Some(next_type) = lookup.return_type_name(&ext_member) {
                let resolved = env.resolve(next_type);
                env.push_scope();
                current_type = resolved;
                continue;
            }
            // The external type is known but this member isn't typed — keep
            // walking with the full external qname so Phase 3 can still match.
            current_type = ext_qname;
            continue;
        }

        // Lost the chain — can't determine the next type.
        return None;
    }

    // Phase 3: Resolve the final segment on the resolved type.
    let last = &segments[segments.len() - 1];

    // Resolve `current_type` to its external qname if it's a short name
    // (e.g., "Assertion" -> "chai.Assertion") so the final lookups hit the
    // package-prefixed symbol rather than returning None.
    let effective_type = external_type_qname(&current_type, lookup)
        .unwrap_or_else(|| current_type.clone());

    let candidate = format!("{effective_type}.{}", last.name);

    // Direct qualified name match.
    if let Some(sym) = lookup.by_qualified_name(&candidate) {
        if kind_compatible(edge_kind, &sym.kind) {
            debug!(
                strategy = "ts_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %effective_type,
                target = %last.name,
                "resolved"
            );
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 1.0,
                strategy: "ts_chain_resolution",
            });
        }
    }

    // Members scoped to the resolved type.
    for sym in lookup.members_of(&effective_type) {
        if sym.name == last.name && kind_compatible(edge_kind, &sym.kind) {
            return Some(Resolution {
                target_symbol_id: sym.id,
                confidence: 0.95,
                strategy: "ts_chain_resolution",
            });
        }
    }

    None
}

/// Find the enclosing class name from the scope chain.
/// scope_chain is `["UserService.findAll", "UserService"]` — we want "UserService".
pub(super) fn find_enclosing_class(
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    for scope in scope_chain {
        if let Some(sym) = lookup.by_qualified_name(scope) {
            if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                return Some(scope.clone());
            }
        }
    }
    // Fallback: the shortest scope entry is often the class.
    scope_chain.last().cloned()
}
