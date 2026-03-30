// =============================================================================
// typescript/chain.rs — TypeScript chain-aware resolution
// =============================================================================

use super::super::super::engine::{RefContext, Resolution, SymbolLookup};
use super::builtins::kind_compatible;
use crate::types::{EdgeKind, MemberChain, SegmentKind};
use tracing::debug;

/// Walk a MemberChain step-by-step, following field types to resolve the final segment.
///
/// For `this.repo.findOne()` with chain `[this, repo, findOne]`:
/// 1. `this` → find enclosing class from scope_chain (e.g., "UserService")
/// 2. `repo` → look up "UserService.repo" field → declared_type = "UserRepo"
/// 3. `findOne` → look up "UserRepo.findOne" in the symbol index → resolved!
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
            let is_type = lookup.by_name(name).iter().any(|s| {
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
    // Track generic type arguments from the field that produced current_type.
    // e.g., for `repo: Repository<User>`, generic_args = ["User"].
    let mut generic_args = initial_generic_args;

    // Phase 2: Walk intermediate segments, following field types or return types.
    for seg in &segments[1..segments.len() - 1] {
        let member_qname = format!("{current_type}.{}", seg.name);

        // Try field type (property access).
        if let Some(next_type) = lookup.field_type_name(&member_qname) {
            // Capture type args from this field for generic substitution.
            generic_args = lookup
                .field_type_args(&member_qname)
                .unwrap_or(&[])
                .to_vec();
            current_type = next_type.to_string();
            continue;
        }

        // Try return type (method call result in a fluent chain).
        if let Some(raw_return) = lookup.return_type_name(&member_qname) {
            // Generic substitution: if return type is a type parameter (e.g., "T"),
            // and we have concrete generic args, substitute.
            let resolved = resolve_generic_type(raw_return, &current_type, &generic_args, lookup);
            generic_args.clear(); // consumed
            current_type = resolved;
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
            debug!(
                strategy = "ts_chain_resolution",
                chain_len = segments.len(),
                resolved_type = %current_type,
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

    // Try by name, scoped to the type.
    for sym in lookup.by_name(&last.name) {
        if sym.qualified_name.starts_with(&current_type) && kind_compatible(edge_kind, &sym.kind) {
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

/// Resolve a generic type parameter to its concrete type.
///
/// If `return_type` is "T" and the enclosing type `Repository` has generic params ["T"]
/// and the field was declared as `Repository<User>` (generic_args = ["User"]),
/// then "T" maps to "User".
///
/// Returns the resolved type name, or the original if no substitution applies.
pub(super) fn resolve_generic_type(
    return_type: &str,
    enclosing_type: &str,
    generic_args: &[String],
    lookup: &dyn SymbolLookup,
) -> String {
    if generic_args.is_empty() {
        return return_type.to_string();
    }
    // Get the generic parameter names for the enclosing type.
    let params = lookup.generic_params(enclosing_type);
    if let Some(params) = params {
        // Find which parameter position matches the return type.
        for (i, param) in params.iter().enumerate() {
            if param == return_type {
                if let Some(concrete) = generic_args.get(i) {
                    return concrete.clone();
                }
            }
        }
    }
    return_type.to_string()
}
