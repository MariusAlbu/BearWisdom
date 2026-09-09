//! Receiver constraints contain TypeIds and GenericParamIds, never type text.
use super::SymbolLookup;
use crate::type_checker::core::types::{
    GenericParamId, Indirection, Lifetime, PrimKind, Type, TypeArena, TypeId,
};
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiverPattern {
    pub(crate) ty: TypeId,
    pub(crate) parameters: Vec<GenericParamId>,
}

impl ReceiverPattern {
    /// Err: unknown applicability; Ok(None): proven mismatch. Successful
    /// bindings are consumed only after every structural constraint agrees.
    pub(crate) fn bindings(
        &self,
        lookup: &dyn SymbolLookup,
        arena: &TypeArena,
        receiver: TypeId,
    ) -> Result<Option<FxHashMap<GenericParamId, TypeId>>, ()> {
        if !lookup.accepts_type_context(arena, self.ty)
            || !lookup.accepts_type_context(arena, receiver)
        {
            return Err(());
        }
        let receiver = expand(lookup, arena, receiver).ok_or(())?;
        let open: FxHashSet<_> = self.parameters.iter().copied().collect();
        let mut bindings = FxHashMap::default();
        let mut pending = vec![(self.ty, receiver, true)];
        let mut unknown = false;
        let mut remaining = 4096usize;
        while let Some((pattern, actual, bind)) = pending.pop() {
            remaining = remaining.checked_sub(1).ok_or(())?;
            match (arena.get(pattern), arena.get(actual)) {
                (Type::Region(Lifetime::Parameter(param)), Type::Region(region))
                    if bind && open.contains(&param) =>
                {
                    if region == Lifetime::Unknown {
                        unknown = true;
                    } else if let Some(old) = bindings.insert(param, actual) {
                        pending.push((old, actual, false));
                    }
                }
                (Type::Generic { param }, _) if bind && open.contains(&param) => {
                    if !super::generic_return::argument_kind_agrees(arena, param, actual) {
                        return Ok(None);
                    }
                    if matches!(
                        arena.get(actual),
                        Type::Unknown | Type::Class(_) | Type::Operator(_)
                    ) {
                        unknown = true;
                    }
                    if let Some(old) = bindings.insert(param, actual) {
                        pending.push((old, actual, false));
                    }
                }
                (Type::Unknown | Type::Class(_), _) | (_, Type::Unknown | Type::Class(_)) => {
                    unknown = true
                }
                (Type::Region(a), Type::Region(b)) => {
                    if a == Lifetime::Unknown || b == Lifetime::Unknown || a != b {
                        unknown = true;
                    }
                }
                (Type::Region(_), _) | (_, Type::Region(_)) => return Ok(None),
                (Type::Generic { param: a }, Type::Generic { param: b }) if a == b => {}
                (Type::Generic { .. }, _) | (_, Type::Generic { .. }) => unknown = true,
                (
                    Type::Decl {
                        symbol_id: a,
                        context: ac,
                        ..
                    },
                    Type::Decl {
                        symbol_id: b,
                        context: bc,
                        ..
                    },
                ) => {
                    if ac != bc || lookup.canonical_decl_id(a) != lookup.canonical_decl_id(b) {
                        return Ok(None);
                    }
                }
                (Type::Primitive(a), Type::Primitive(b)) => {
                    if matches!(a, PrimKind::Int | PrimKind::Float | PrimKind::Unknown)
                        || matches!(b, PrimKind::Int | PrimKind::Float | PrimKind::Unknown)
                    {
                        unknown = true;
                    } else if a != b {
                        return Ok(None);
                    }
                }
                (Type::Apply { base: a, args: ax }, Type::Apply { base: b, args: bx }) => {
                    if ax.len() != bx.len() {
                        return Ok(None);
                    }
                    pending.push((a, b, bind));
                    pending.extend(ax.into_iter().zip(bx).map(|(a, b)| (a, b, bind)));
                }
                (Type::Tuple(ax), Type::Tuple(bx)) => {
                    if ax.len() != bx.len() {
                        return Ok(None);
                    }
                    pending.extend(ax.into_iter().zip(bx).map(|(a, b)| (a, b, bind)));
                }
                (
                    Type::Indirect {
                        kind: a,
                        mutability: am,
                        inner: ax,
                    },
                    Type::Indirect {
                        kind: b,
                        mutability: bm,
                        inner: bx,
                    },
                ) => {
                    if am != bm
                        || matches!(
                            (a, b),
                            (Indirection::Pointer, Indirection::Reference(_))
                                | (Indirection::Reference(_), Indirection::Pointer)
                        )
                    {
                        return Ok(None);
                    }
                    if let (Indirection::Reference(a), Indirection::Reference(b)) = (a, b) {
                        pending.push((
                            arena.intern(Type::Region(a)),
                            arena.intern(Type::Region(b)),
                            bind,
                        ));
                    }
                    pending.push((ax, bx, bind));
                }
                (
                    Type::Indirect { .. },
                    Type::Decl { .. } | Type::Apply { .. } | Type::Primitive(_) | Type::Tuple(_),
                )
                | (
                    Type::Decl { .. } | Type::Apply { .. } | Type::Primitive(_) | Type::Tuple(_),
                    Type::Indirect { .. },
                ) => return Ok(None),
                // A bare nominal may have unapplied arguments, not a proven
                // disjoint application. Do not select a specialization on it.
                (Type::Apply { .. }, Type::Decl { .. })
                | (Type::Decl { .. }, Type::Apply { .. }) => unknown = true,
                (Type::Literal(a), Type::Literal(b)) => {
                    if a != b {
                        return Ok(None);
                    }
                }
                _ => unknown = true,
            }
        }
        if unknown || open.iter().any(|id| !bindings.contains_key(id)) {
            Err(())
        } else {
            Ok(Some(bindings))
        }
    }
}

/// Expand only attested alias templates. Numeric declaration IDs delimit cycles
/// even for aliases that grow their applications at each recursive step.
pub(crate) fn expand(lookup: &dyn SymbolLookup, arena: &TypeArena, ty: TypeId) -> Option<TypeId> {
    if !lookup.accepts_type_context(arena, ty) {
        return None;
    }
    if let Some(result) = lookup.evaluated_receiver(ty) {
        return result.filter(|&ty| lookup.accepts_type_context(arena, ty));
    }
    expand_inner(lookup, arena, ty, &mut FxHashSet::default(), &mut 4096, 0)
}

fn expand_inner(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    ty: TypeId,
    active: &mut FxHashSet<i64>,
    remaining: &mut usize,
    depth: usize,
) -> Option<TypeId> {
    if !lookup.accepts_type_context(arena, ty) {
        return None;
    }
    *remaining = remaining.checked_sub(1)?;
    if depth > 32 {
        return None;
    }
    let head = match arena.get(ty) {
        Type::Apply { base, .. } => base,
        _ => ty,
    };
    if let Type::Decl { symbol_id, .. } = arena.get(head) {
        let id = lookup.canonical_decl_id(symbol_id);
        let info = lookup.canonical_type_info(id);
        if let Some(template) = info.and_then(|info| info.lexical_alias.as_ref()) {
            let args = super::super::chain::apply_args(arena, ty);
            // Argument aliases are siblings, not recursive expansion of this alias.
            let args = args
                .into_iter()
                .map(|arg| expand_inner(lookup, arena, arg, active, remaining, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            if !active.insert(id) {
                return None;
            }
            let result = template
                .instantiate(arena, &args)
                .and_then(|ty| expand_inner(lookup, arena, ty, active, remaining, depth + 1));
            active.remove(&id);
            return result;
        }
        if lookup
            .symbol_by_id(id)
            .is_some_and(|s| s.kind == "type_alias")
        {
            return None;
        }
    }
    let result = match arena.get(ty) {
        Type::Apply { base, args } => {
            let base = expand_inner(lookup, arena, base, active, remaining, depth + 1)?;
            let args = args
                .into_iter()
                .map(|arg| expand_inner(lookup, arena, arg, active, remaining, depth + 1))
                .collect::<Option<Vec<_>>>()?;
            Type::Apply { base, args }
        }
        Type::Tuple(items) => Type::Tuple(
            items
                .into_iter()
                .map(|arg| expand_inner(lookup, arena, arg, active, remaining, depth + 1))
                .collect::<Option<Vec<_>>>()?,
        ),
        Type::Indirect {
            kind,
            mutability,
            inner,
        } => Type::Indirect {
            kind,
            mutability,
            inner: expand_inner(lookup, arena, inner, active, remaining, depth + 1)?,
        },
        other => other,
    };
    Some(arena.intern(result))
}

#[cfg(test)]
#[path = "member_applicability_tests.rs"]
mod tests;
