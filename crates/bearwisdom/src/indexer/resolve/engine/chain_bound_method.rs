//! Selected methods bypass name-based yield and overload retries.
use super::*;
use crate::indexer::resolve::engine::contract::flow_cache::BoundMethod;

pub(super) fn call(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
    segment: &crate::types::ChainSegment,
    terminal_args: Option<&Vec<crate::types::CallArg>>,
    profile: &LanguageProfile,
) -> Option<Result<SymbolInfo, ()>> {
    let (receiver, nullable) = if segment.optional_chaining && lookup.nominal_context().is_some() {
        match optional_receiver(lookup, arena, receiver) {
            Some(value) => value,
            None => return Some(Err(())),
        }
    } else {
        (receiver, false)
    };
    let selected = match lookup.bound_method(receiver, segment.byte_offset)? {
        Ok(selected) => Some(Ok(apply(
            lookup,
            arena,
            &selected,
            segment,
            terminal_args,
            profile,
        ))),
        Err(()) => Some((|| {
            let args = super::super::arg_types::at(
                lookup,
                segment.byte_offset,
                terminal_args.unwrap_or(&segment.call_args),
            )
            .ok_or(())?;
            let actual = resolve_arg_types(lookup, arena, args);
            let explicit = super::super::segment_args::type_arguments(lookup, arena, segment);
            let selected = lookup
                .overloaded_call(receiver, segment.byte_offset, &actual, &explicit)
                .ok_or(())??;
            let origin = selected.origins.get(selected.selected).ok_or(())?;
            let declaration = origin
                .declaration
                .filter(|&id| lookup.symbol_by_id(id).is_some())
                .ok_or(())?;
            super::super::lambda_seed::seed_patterns(
                lookup,
                arena,
                args,
                &selected.parameters,
                &Default::default(),
                profile.delegate_wrappers,
            );
            Ok(SymbolInfo {
                target_symbol_id: declaration,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "bound_overload",
                resolved_yield_type: Some(selected.return_type),
                flow_emit: None,
            })
        })()),
    };
    selected.map(|result| {
        result.map(|mut info| {
            if nullable {
                info.resolved_yield_type = info
                    .resolved_yield_type
                    .map(|ty| arena.intern(Type::Optional(ty)));
            }
            info
        })
    })
}

/// Optional calls select members on the present receiver but retain the
/// short-circuit alternative in their result. Never erase a non-null branch.
fn optional_receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: TypeId,
) -> Option<(TypeId, bool)> {
    use crate::type_checker::core::types::Intrinsic;
    let receiver = super::super::contract::member_applicability::expand(lookup, arena, receiver)?;
    let mut pending = vec![receiver];
    let mut present = Vec::new();
    let mut nullable = false;
    let mut remaining = 4096usize;
    while let Some(ty) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        match arena.get(ty) {
            Type::Optional(inner) => {
                nullable = true;
                pending.push(inner);
            }
            Type::Union(parts) => pending.extend(parts),
            Type::Intrinsic(Intrinsic::Null | Intrinsic::Undefined) => nullable = true,
            Type::Unknown
            | Type::Class(_)
            | Type::Intrinsic(Intrinsic::Unknown | Intrinsic::Any | Intrinsic::Never) => {
                return None
            }
            _ => {
                if !present.contains(&ty) {
                    present.push(ty);
                }
            }
        }
    }
    // Multiple actual receivers require union-member agreement, not a chosen
    // first branch. That separate proof remains outside this adapter.
    match present.as_slice() {
        [ty] => Some((*ty, nullable)),
        _ => None,
    }
}

/// Uncalled selectors use the same source IDs as calls. An attested miss is
/// final, not permission to retry display names, implicit roots or overloads.
pub(super) fn member(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    receiver: Receiver,
    segment: &crate::types::ChainSegment,
    profile: &LanguageProfile,
) -> Result<Option<Symbol>, super::super::member_selection::Selection> {
    use super::super::member_selection::{select_typed, Selection};
    if let Some(member) = lookup.source_object_member(receiver.ty, segment.byte_offset) {
        return member
            .map_err(|_| Selection::Missing)?
            .declaration
            .and_then(|row| lookup.symbol_by_id(row))
            .cloned()
            .map(Some)
            .ok_or(Selection::Missing);
    }
    if let Some(declaration) = lookup.source_private_member(segment.byte_offset) {
        let selected = super::super::program_view::select_private(
            lookup,
            arena,
            receiver.ty,
            declaration.map_err(|_| Selection::Missing)?,
        )
        .map_err(|_| Selection::Inaccessible)?;
        return lookup
            .symbol_by_id(selected.declaration)
            .cloned()
            .map(Some)
            .ok_or(Selection::Missing);
    }
    let Some(name) = lookup.source_member_name(segment.byte_offset) else {
        return implicit_root::walk_member(lookup, arena, receiver, &segment.name, profile);
    };
    match select_typed(
        lookup,
        arena,
        receiver,
        name.map_err(|_| Selection::Missing)?,
        &|_| true,
    ) {
        Selection::Unique(id) => lookup
            .symbol_by_id(id)
            .cloned()
            .map(Some)
            .ok_or(Selection::Missing),
        denied => Err(denied),
    }
}

/// Preserve the receiver's reference layers while carrying the declaration ID
/// already established by root/field projection. Never search a head by spelling.
pub(super) fn receiver(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    original: TypeId,
    projected: Receiver,
) -> TypeId {
    let mut pointee = original;
    for _ in 0..32 {
        match arena.get(pointee) {
            Type::Indirect {
                kind: crate::type_checker::core::types::Indirection::Reference(_),
                inner,
                ..
            } => pointee = inner,
            _ => break,
        }
    }
    // A wrapper/alias projection can change the actual receiver, not merely
    // reveal its head. Its declaration must never be stamped onto the old
    // application's arguments (Box<Doc> must not become Doc<Doc>).
    if projected.ty != pointee {
        return original;
    }
    fn bind(arena: &TypeArena, ty: TypeId, declaration: TypeId, depth: usize) -> TypeId {
        if depth > 32 {
            return arena.intern(Type::Unknown);
        }
        arena.intern(match arena.get(ty) {
            Type::Class(_) => return declaration,
            Type::Apply { base, args } => Type::Apply {
                base: bind(arena, base, declaration, depth + 1),
                args,
            },
            Type::Indirect {
                kind,
                mutability,
                inner,
            } => Type::Indirect {
                kind,
                mutability,
                inner: bind(arena, inner, declaration, depth + 1),
            },
            other => other,
        })
    }
    let Some(symbol) = projected.id.and_then(|id| lookup.symbol_by_id(id)) else {
        return original;
    };
    bind(
        arena,
        original,
        arena.decl(&symbol.qualified_name, lookup.canonical_decl_id(symbol.id)),
        0,
    )
}

pub(super) fn apply(
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    selected: &BoundMethod,
    segment: &crate::types::ChainSegment,
    terminal_args: Option<&Vec<crate::types::CallArg>>,
    profile: &LanguageProfile,
) -> SymbolInfo {
    let legacy = terminal_args.unwrap_or(&segment.call_args);
    let resolved_yield_type = lookup
        .symbol_by_id(selected.declaration)
        .and_then(|member| {
            let args = super::super::arg_types::at(lookup, segment.byte_offset, legacy)?;
            let explicit = super::super::segment_args::type_arguments(lookup, arena, segment);
            let actual = resolve_arg_types(lookup, arena, args);
            let env = super::super::bound_call::environment_with_initial(
                lookup,
                arena,
                member,
                selected.receiver,
                super::super::head_decl::head_decl_id(arena, selected.receiver),
                &explicit,
                &actual,
                Some(selected.adjusted),
                &selected.bindings,
            )?;
            let info = lookup.member_info(arena, selected.receiver, selected.declaration)?;
            let rewrite = |ty| super::super::contract::generic_return::substitute(arena, ty, &env);
            let patterns: Vec<_> = info
                .parameter_type_ids
                .as_ref()?
                .iter()
                .map(|&ty| rewrite(ty))
                .collect();
            super::super::lambda_seed::seed_patterns(
                lookup,
                arena,
                args,
                &patterns,
                &Default::default(),
                profile.delegate_wrappers,
            );
            info.return_type_id.map(rewrite)
        });
    SymbolInfo {
        target_symbol_id: selected.declaration,
        confidence: RESOLVED_CONFIDENCE,
        strategy: "bound_method",
        resolved_yield_type,
        flow_emit: None,
    }
}

#[cfg(test)]
#[path = "chain_bound_method_tests.rs"]
mod tests;
