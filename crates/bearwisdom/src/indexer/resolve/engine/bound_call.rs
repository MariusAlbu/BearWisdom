//! Source-bound call signatures. Runtime substitution keys only GenericParamId.
use super::contract::generic_return::{argument_kind_agrees, substitute};
use super::contract::{Symbol, SymbolLookup};
use crate::type_checker::core::types::{
    GenericParamId, GenericParamKind, Indirection, Lifetime, Type, TypeArena, TypeId,
};
use rustc_hash::{FxHashMap, FxHashSet};

pub(super) type Bindings = FxHashMap<GenericParamId, TypeId>;

pub(super) fn member_yield(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    member: i64,
    receiver: TypeId,
    ty: TypeId,
) -> TypeId {
    if !lookup.accepts_type_context(arena, receiver) || !lookup.accepts_type_context(arena, ty) {
        return arena.intern(Type::Unknown);
    }
    let Some(pattern) = lookup.member_pattern(member) else {
        return ty;
    };
    match pattern.bindings(lookup, arena, receiver) {
        Ok(Some(bindings)) => substitute(arena, ty, &bindings),
        _ => arena.intern(Type::Unknown),
    }
}

pub(super) fn receiver_bindings(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    receiver_id: Option<i64>,
) -> Bindings {
    if !lookup.accepts_type_context(arena, receiver) {
        return Bindings::default();
    }
    let Some(id) = super::head_decl::head_decl_id(arena, receiver).or(receiver_id) else {
        return Bindings::default();
    };
    let Some(info) = lookup.canonical_type_info(id) else {
        return Bindings::default();
    };
    let args = super::chain::apply_args(arena, receiver);
    let mut bindings: Bindings = info
        .generic_param_ids
        .iter()
        .copied()
        .zip(args.iter().copied())
        .collect();
    for (index, &parameter) in info.generic_param_ids.iter().enumerate().skip(args.len()) {
        let Some(default) = info.generic_param_default_ids.get(index).copied().flatten() else {
            break;
        };
        let argument = substitute(arena, default, &bindings);
        if !lookup.accepts_type_context(arena, argument)
            || !argument_kind_agrees(arena, parameter, argument)
        {
            break;
        }
        bindings.insert(parameter, argument);
    }
    bindings
}

pub(super) fn explicit(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: i64,
    arguments: &[TypeId],
    yielded: TypeId,
) -> Option<TypeId> {
    if !lookup.accepts_type_context(arena, yielded)
        || arguments
            .iter()
            .any(|&ty| !lookup.accepts_type_context(arena, ty))
    {
        return Some(arena.intern(Type::Unknown));
    }
    let info = lookup.canonical_type_info(callee)?;
    info.parameter_type_ids.as_ref()?;
    if arguments.len() > info.generic_param_ids.len()
        || info
            .generic_param_ids
            .iter()
            .zip(arguments)
            .any(|(&p, &a)| !argument_kind_agrees(arena, p, a))
    {
        return Some(arena.intern(Type::Unknown));
    }
    let bindings = info
        .generic_param_ids
        .iter()
        .copied()
        .zip(arguments.iter().copied())
        .collect();
    Some(substitute(arena, yielded, &bindings))
}

pub(super) fn environment(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    receiver: TypeId,
    receiver_id: Option<i64>,
    explicit: &[TypeId],
    actual: &[TypeId],
) -> Option<Bindings> {
    environment_with_receiver(
        lookup,
        arena,
        callee,
        receiver,
        receiver_id,
        explicit,
        actual,
        None,
    )
}

pub(super) fn environment_with_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    receiver: TypeId,
    receiver_id: Option<i64>,
    explicit: &[TypeId],
    actual: &[TypeId],
    borrowed: Option<TypeId>,
) -> Option<Bindings> {
    environment_with_initial(
        lookup,
        arena,
        callee,
        receiver,
        receiver_id,
        explicit,
        actual,
        borrowed,
        &Bindings::default(),
    )
}

pub(super) fn environment_with_initial(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    callee: &Symbol,
    receiver: TypeId,
    receiver_id: Option<i64>,
    explicit: &[TypeId],
    actual: &[TypeId],
    borrowed: Option<TypeId>,
    initial: &Bindings,
) -> Option<Bindings> {
    if std::iter::once(&receiver)
        .chain(explicit)
        .chain(actual)
        .chain(borrowed.iter())
        .chain(initial.values())
        .any(|&ty| !lookup.accepts_type_context(arena, ty))
    {
        return None;
    }
    let info = lookup.member_info(arena, receiver, callee.id)?;
    let patterns = info.parameter_type_ids.as_ref()?;
    if patterns
        .iter()
        .chain(info.receiver_type_id.iter())
        .any(|&ty| !lookup.accepts_type_context(arena, ty))
    {
        return None;
    }
    let mut bindings = if lookup
        .projected_member_info(arena, receiver, callee.id)
        .is_none()
        && super::inherited_bindings::captured(lookup, arena, receiver, receiver_id)
    {
        super::inherited_bindings::for_member(lookup, arena, receiver, receiver_id, callee.id)?
    } else {
        receiver_bindings(lookup, arena, receiver, receiver_id)
    };
    bindings.extend(initial.iter().map(|(&p, &ty)| (p, ty)));
    if let Some(pattern) = lookup.member_pattern(callee.id) {
        bindings.extend(pattern.bindings(lookup, arena, receiver).ok()??);
    }
    // Only the callee's own IDs are inferable. A still-open receiver parameter
    // is not an invitation to infer a different instantiation from this call.
    let open: FxHashSet<_> = info
        .generic_param_ids
        .iter()
        .copied()
        .chain(info.elided_input_params.iter().map(|&(_, _, p)| p))
        .collect();
    if explicit.len() > info.generic_param_ids.len()
        || info
            .generic_param_ids
            .iter()
            .zip(explicit)
            .any(|(&p, &a)| !argument_kind_agrees(arena, p, a))
    {
        return None;
    }
    for (&id, &ty) in info.generic_param_ids.iter().zip(explicit) {
        bindings.insert(id, ty);
    }
    let mut inferred = Bindings::default();
    let mut conflicts = FxHashSet::default();
    for (&pattern, &ty) in patterns.iter().zip(actual) {
        infer(
            arena,
            substitute(arena, pattern, &bindings),
            ty,
            &open,
            &mut inferred,
            &mut conflicts,
        );
    }
    if let (Some(pattern), Some(actual)) = (info.receiver_type_id, borrowed) {
        let pattern = substitute(arena, pattern, &bindings);
        // Alias-impl Self parameters are owned by the impl, not the nominal.
        let pattern = member_yield(lookup, arena, callee.id, receiver, pattern);
        if let (Type::Indirect { inner: p, .. }, Type::Indirect { inner: a, .. }) =
            (arena.get(pattern), arena.get(actual))
        {
            if let Some(p) = super::contract::member_applicability::expand(lookup, arena, p) {
                let exact = super::contract::member_applicability::ReceiverPattern {
                    ty: p,
                    parameters: vec![],
                };
                if matches!(exact.bindings(lookup, arena, a), Ok(Some(_))) {
                    infer(arena, pattern, actual, &open, &mut inferred, &mut conflicts);
                }
            }
        }
    }
    for (id, ty) in inferred {
        if !conflicts.contains(&id) {
            bindings.entry(id).or_insert(ty);
        }
    }
    for (index, &id) in info.generic_param_ids.iter().enumerate() {
        if !bindings.contains_key(&id) && !conflicts.contains(&id) {
            if let Some(Some(default)) = info.generic_param_default_ids.get(index) {
                bindings.insert(id, substitute(arena, *default, &bindings));
            }
        }
        if arena.generic_kind(id) == GenericParamKind::Lifetime {
            bindings
                .entry(id)
                .or_insert_with(|| arena.intern(Type::Region(Lifetime::Unknown)));
        }
    }
    for &(_, _, id) in &info.elided_input_params {
        bindings
            .entry(id)
            .or_insert_with(|| arena.intern(Type::Region(Lifetime::Unknown)));
    }
    crate::tracef!(
        "  BOUND-ARGS callee={} patterns={:?} actual={:?} bindings={:?} return={:?}",
        callee.id,
        patterns,
        actual,
        bindings,
        info.return_type_id
    );
    crate::tracef!(
        "  BOUND-OWNER receiver_type={:?} receiver_owner={:?} callee_owner={:?}",
        arena.get(receiver),
        super::head_decl::head_decl_id(arena, receiver)
            .or(receiver_id)
            .map(|id| (id, lookup.symbol_by_id(id).map(|s| &s.file_path))),
        lookup
            .enclosing_type_id_of(callee.id)
            .map(|id| (id, lookup.symbol_by_id(id).map(|s| &s.file_path)))
    );
    Some(bindings)
}

/// No formatting or nominal spelling comparison: constructor matches use the
/// bound declaration ID (or the exact interned structural head for legacy data).
fn same_head(arena: &TypeArena, left: TypeId, right: TypeId) -> bool {
    match (
        super::head_decl::head_identity(arena, left),
        super::head_decl::head_identity(arena, right),
    ) {
        (Some(a), Some(b)) => a == b,
        (None, None) => left == right,
        _ => false,
    }
}

fn infer(
    arena: &TypeArena,
    pattern: TypeId,
    actual: TypeId,
    open: &FxHashSet<GenericParamId>,
    bindings: &mut Bindings,
    conflicts: &mut FxHashSet<GenericParamId>,
) {
    if matches!(
        arena.get(actual),
        Type::Unknown | Type::Generic { .. } | Type::Region(Lifetime::Unknown)
    ) {
        return;
    }
    match (arena.get(pattern), arena.get(actual)) {
        (Type::Generic { param } | Type::Region(Lifetime::Parameter(param)), _)
            if open.contains(&param) && argument_kind_agrees(arena, param, actual) =>
        {
            if bindings.get(&param).is_some_and(|old| *old != actual) {
                conflicts.insert(param);
            } else {
                bindings.insert(param, actual);
            }
        }
        (Type::Apply { base: p, args: ps }, Type::Apply { base: a, args: xs })
            if ps.len() == xs.len() && same_head(arena, p, a) =>
        {
            for (p, a) in ps.into_iter().zip(xs) {
                infer(arena, p, a, open, bindings, conflicts);
            }
        }
        (
            Type::Function {
                params: ps,
                return_: pr,
            },
            Type::Function {
                params: xs,
                return_: ar,
            },
        ) if ps.len() == xs.len() => {
            for (p, a) in ps.into_iter().zip(xs) {
                infer(arena, p, a, open, bindings, conflicts);
            }
            infer(arena, pr, ar, open, bindings, conflicts);
        }
        (Type::Tuple(ps), Type::Tuple(xs)) if ps.len() == xs.len() => {
            for (p, a) in ps.into_iter().zip(xs) {
                infer(arena, p, a, open, bindings, conflicts);
            }
        }
        (Type::Optional(p), Type::Optional(a))
        | (Type::AsyncWrapper(p), Type::AsyncWrapper(a))
        | (Type::Iterator(p), Type::Iterator(a))
        | (Type::Constructor(p), Type::Constructor(a)) => {
            infer(arena, p, a, open, bindings, conflicts);
        }
        (Type::Optional(p), _) => infer(arena, p, actual, open, bindings, conflicts),
        // Exact region correspondence, not an outlives/borrow proof. Conflicting
        // occurrences stay unbound; projection must not erase argument facts.
        (
            Type::Indirect {
                kind: p,
                mutability: pm,
                inner: pi,
            },
            Type::Indirect {
                kind: a,
                mutability: am,
                inner: ai,
            },
        ) if pm == am => match (p, a) {
            (Indirection::Reference(p), Indirection::Reference(a)) => {
                infer(
                    arena,
                    arena.intern(Type::Region(p)),
                    arena.intern(Type::Region(a)),
                    open,
                    bindings,
                    conflicts,
                );
                infer(arena, pi, ai, open, bindings, conflicts);
            }
            (Indirection::Pointer, Indirection::Pointer) => {
                infer(arena, pi, ai, open, bindings, conflicts)
            }
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
#[path = "bound_call_tests.rs"]
mod tests;
