//! A source-bound lexical root never enters a global name-search ladder.
use super::*;
use crate::indexer::resolve::engine::contract::flow_cache::LocalReference;
use crate::types::{ChainSegment, SymbolKind};

#[cfg(test)]
#[path = "chain_lexical_root_tests.rs"]
mod tests;

pub(super) fn resolve(
    local: LocalReference,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    file: &FileContext,
    segment: &ChainSegment,
    profile: &LanguageProfile,
) -> Result<Receiver, Option<Cause>> {
    if !segment.is_call
        && (matches!(
            local.kind,
            SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Interface
        ) || (local.kind == SymbolKind::TypeAlias && local.value_type.is_some()))
    {
        let symbol = local
            .declaration
            .and_then(|id| lookup.symbol_by_id(id))
            .ok_or(None)?;
        let mut ty = super::super::head_decl::nominal_head(lookup, arena, symbol);
        if !segment.type_arg_ids.is_empty() {
            ty = arena.intern(Type::Apply {
                base: ty,
                args: segment.type_arg_ids.clone(),
            });
        }
        return Ok(Receiver::new(ty, symbol.id));
    }
    if local.kind == SymbolKind::Class {
        let symbol = local
            .declaration
            .and_then(|id| lookup.symbol_by_id(id))
            .ok_or(None)?;
        let ty =
            super::super::lexical_value::yield_type(&local, lookup, arena, true).ok_or(None)?;
        return Ok(Receiver::new(ty, symbol.id));
    }
    let callable = if local.kind == SymbolKind::Function {
        local.declaration
    } else {
        local.callable
    };
    if segment.is_call {
        let (mut ty, signature) =
            match super::super::lexical_value::yield_type(&local, lookup, arena, false) {
                Some(ty) => (ty, callable),
                // The binding names no callable whose return was captured, but
                // the value it binds may still carry a call signature on its own
                // TYPE — an inline signature, or a declaration whose call
                // signature the extractor surfaced. Calling it yields that
                // signature's return.
                None => {
                    let yielded = [
                        carried_type(&local, arena),
                        declared_type(&local, lookup, arena),
                    ]
                    .into_iter()
                    .flatten()
                    .find_map(|ty| super::callable_value::call_yield(lookup, arena, ty))
                    .ok_or(Some(Cause::new(
                        callable.or(local.declaration),
                        CauseKind::UncapturedReturn,
                    )))?;
                    (yielded.ty, yielded.signature_id)
                }
            };
        if let Some(callee) = signature.and_then(|id| lookup.symbol_by_id(id)) {
            ty = bind_call_args_into_return(
                lookup,
                arena,
                callee.id,
                segment.byte_offset,
                &segment.call_args,
                ty,
            );
        }
        return Ok(Receiver::untyped(ty));
    }
    // A binding that carries no type of its own still roots on the declared
    // type of the declaration it names — the same evidence a root bound by name
    // reads, which a source-bound root would otherwise never reach.
    let ty = local
        .value_type
        .or_else(|| declared_type(&local, lookup, arena))
        .ok_or(Some(Cause::new(
            local.declaration,
            CauseKind::UntypedBinding,
        )))?;
    if matches!(arena.get(ty), Type::Constructor(_)) {
        return super::super::lexical_value::yield_type(&local, lookup, arena, true)
            .map(Receiver::untyped)
            .ok_or(None);
    }
    Ok(Receiver::untyped(
        resolve_return_type_extraction(ty, lookup, arena, file, Some(profile)).unwrap_or(ty),
    ))
}

/// The type the binding itself was given. A slot holding `Unknown` carries no
/// type, so it reads as absent.
fn carried_type(local: &LocalReference, arena: &TypeArena) -> Option<TypeId> {
    local
        .value_type
        .filter(|&ty| !matches!(arena.get(ty), Type::Unknown))
}

/// The declared type on the VALUE declaration the binding names — the type a
/// binding inherits when it carries none of its own, the way a root bound by
/// name reads its declaration.
fn declared_type(
    local: &LocalReference,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
) -> Option<TypeId> {
    let declaration = lookup.symbol_by_id(local.declaration?)?;
    if !is_value_kind(&declaration.kind) {
        return None;
    }
    field_type_of(
        lookup,
        arena,
        declaration.id,
        &declaration.qualified_name,
    )
    .filter(|&ty| !matches!(arena.get(ty), Type::Unknown))
}
