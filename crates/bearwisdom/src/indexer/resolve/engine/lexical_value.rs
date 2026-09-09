//! Call/constructor yields for already-bound values. No name lookup or reparsing.
use super::contract::{flow_cache::LocalReference, SymbolLookup};
use crate::type_checker::core::types::{Type, TypeArena, TypeId};
use crate::types::SymbolKind;

/// Preserve every source-declared signature. This does not select an overload.
pub(super) fn imported_overload_type(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    path: &str,
    binding: crate::indexer::lexical::BindingId,
) -> Option<TypeId> {
    let ids = lookup.bound_import_overloads(path, binding);
    if ids.is_empty() {
        return None;
    }
    let signatures: Option<Vec<_>> = ids
        .iter()
        .map(|&id| {
            let info = lookup.canonical_type_info(id)?;
            Some(
                arena.intern(Type::Function {
                    params: info.parameter_type_ids.clone()?,
                    return_: info
                        .return_type_id
                        .unwrap_or_else(|| arena.intern(Type::Unknown)),
                }),
            )
        })
        .collect();
    signatures.map(|signatures| arena.intern(Type::Intersection(signatures)))
}

pub(super) fn callable_return(arena: &TypeArena, ty: TypeId) -> Option<TypeId> {
    match arena.get(ty) {
        Type::Function { return_, .. } => Some(return_),
        Type::Intersection(signatures) if !signatures.is_empty() => {
            let mut returns = Vec::new();
            for signature in signatures {
                let Type::Function { return_, .. } = arena.get(signature) else {
                    return None;
                };
                if matches!(arena.get(return_), Type::Unknown) {
                    return Some(return_);
                }
                returns.push(return_);
            }
            returns.sort_unstable();
            returns.dedup();
            Some(if returns.len() == 1 {
                returns[0]
            } else {
                arena.intern(Type::Union(returns))
            })
        }
        _ => None,
    }
}

/// Type of a bound value, not the value's call result. No name/ID recovery.
pub(super) fn argument_type(
    local: &LocalReference,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
) -> Option<TypeId> {
    if local.kind == SymbolKind::Class {
        let symbol = lookup.symbol_by_id(local.declaration?)?;
        return Some(
            arena.intern(Type::Constructor(super::head_decl::nominal_head(
                lookup, arena, symbol,
            ))),
        );
    }
    if let Some(ty) = local.value_type {
        return Some(ty);
    }
    let callable = if local.kind == SymbolKind::Function {
        local.declaration
    } else {
        local.callable
    }?;
    let info = lookup.canonical_type_info(callable)?;
    Some(
        arena.intern(Type::Function {
            params: info.parameter_type_ids.clone()?,
            return_: info
                .return_type_id
                .unwrap_or_else(|| arena.intern(Type::Unknown)),
        }),
    )
}

pub(super) fn yield_type(
    local: &LocalReference,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    constructing: bool,
) -> Option<TypeId> {
    if constructing {
        let base = if local.kind == SymbolKind::Class {
            let symbol = lookup.symbol_by_id(local.declaration?)?;
            super::head_decl::nominal_head(lookup, arena, symbol)
        } else if let Type::Constructor(instance) = arena.get(local.value_type?) {
            instance
        } else {
            return None;
        };
        return Some(if local.type_args.is_empty() {
            base
        } else {
            arena.intern(Type::Apply {
                base,
                args: local.type_args.clone(),
            })
        });
    }
    // A value's declared callable contract wins over initializer provenance.
    if let Some(return_) = local.value_type.and_then(|ty| callable_return(arena, ty)) {
        return Some(return_);
    }
    let callable = if local.kind == SymbolKind::Function {
        local.declaration
    } else {
        local.callable
    }?;
    if local.type_args.is_empty() {
        lookup.return_type_id_of(callable)
    } else {
        lookup
            .generic_return_of(callable)?
            .instantiate(arena, &local.type_args)
    }
}

#[cfg(test)]
#[path = "lexical_value_tests.rs"]
mod tests;
