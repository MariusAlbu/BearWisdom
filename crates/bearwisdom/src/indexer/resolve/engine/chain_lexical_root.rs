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
        let mut ty =
            super::super::lexical_value::yield_type(&local, lookup, arena, false).ok_or(Some(
                Cause::new(callable.or(local.declaration), CauseKind::UncapturedReturn),
            ))?;
        if let Some(callee) = callable.and_then(|id| lookup.symbol_by_id(id)) {
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
    let ty = local.value_type.ok_or(Some(Cause::new(
        local.declaration,
        CauseKind::UntypedBinding,
    )))?;
    if matches!(arena.get(ty), Type::Constructor(_)) {
        return super::super::lexical_value::yield_type(&local, lookup, arena, true)
            .map(Receiver::untyped)
            .ok_or(None);
    }
    Ok(Receiver::untyped(
        resolve_return_type_extraction(ty, lookup, arena, file).unwrap_or(ty),
    ))
}
