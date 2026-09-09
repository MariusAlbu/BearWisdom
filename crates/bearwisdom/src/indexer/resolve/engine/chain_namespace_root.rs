//! Namespace prefixes select source-addressed export IDs before the value walk.
use super::*;
use crate::types::SymbolKind;

pub(super) enum Anchor {
    Terminal(SymbolInfo),
    Receiver(Receiver, usize),
}

pub(super) fn anchor(
    ref_ctx: &RefContext,
    file: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Option<Result<Anchor, Option<Cause>>> {
    let root = lookup.local_reference(ref_ctx.extracted_ref.byte_offset)?;
    if ref_ctx
        .extracted_ref
        .chain
        .as_ref()
        .and_then(|chain| chain.segments.first())
        .is_some_and(|s| s.is_call)
    {
        return None;
    }
    if root.kind != SymbolKind::Namespace
        && !lookup.namespace_root(ref_ctx.extracted_ref.byte_offset)
    {
        return None;
    }
    Some(bind(ref_ctx, file, lookup, arena, profile))
}

fn bind(
    ref_ctx: &RefContext,
    file: &FileContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Result<Anchor, Option<Cause>> {
    let chain = ref_ctx.extracted_ref.chain.as_ref().ok_or(None)?;
    for (index, segment) in chain.segments.iter().enumerate().skip(1) {
        let local = lookup.namespace_member(segment.byte_offset).ok_or(None)?;
        if local.kind == SymbolKind::Namespace {
            if segment.is_call {
                return Err(None);
            }
            continue;
        }
        if index + 1 == chain.segments.len() {
            let id = local.declaration.ok_or(None)?;
            let ty = call_yield(
                &local,
                lookup,
                arena,
                segment.byte_offset,
                &ref_ctx.extracted_ref.call_args,
                ref_ctx.extracted_ref.kind == crate::types::EdgeKind::Instantiates,
                profile,
            );
            return Ok(Anchor::Terminal(SymbolInfo {
                target_symbol_id: id,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "namespace_binding",
                resolved_yield_type: ty,
                flow_emit: None,
            }));
        }
        if segment.is_call && local.kind != SymbolKind::Class {
            return call_yield(
                &local,
                lookup,
                arena,
                segment.byte_offset,
                &segment.call_args,
                false,
                profile,
            )
            .map(|ty| Anchor::Receiver(Receiver::untyped(ty), index + 1))
            .ok_or(None);
        }
        return lexical_root::resolve(local, lookup, arena, file, segment)
            .map(|root| Anchor::Receiver(root, index + 1));
    }
    Err(None)
}

fn call_yield(
    local: &crate::indexer::resolve::engine::contract::flow_cache::LocalReference,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    selector: u32,
    args: &[crate::types::CallArg],
    constructing: bool,
    profile: &LanguageProfile,
) -> Option<TypeId> {
    let args = super::super::arg_types::at(lookup, selector, args)?;
    let yielded = super::super::lexical_value::yield_type(local, lookup, arena, constructing);
    if constructing {
        return yielded;
    }
    let fallback = || {
        if lookup.source_call_arguments(selector).is_some() {
            None
        } else {
            yielded
        }
    };
    let Some(callee) = local.declaration.and_then(|id| lookup.symbol_by_id(id)) else {
        return fallback();
    };
    let actual = resolve_arg_types(lookup, arena, args);
    let Some(env) = super::super::bound_call::environment(
        lookup,
        arena,
        callee,
        arena.intern(Type::Unknown),
        None,
        &local.type_args,
        &actual,
    ) else {
        return fallback();
    };
    let rewrite = |ty| super::super::contract::generic_return::substitute(arena, ty, &env);
    let patterns: Vec<_> = lookup
        .canonical_type_info(callee.id)
        .and_then(|i| i.parameter_type_ids.as_ref())
        .into_iter()
        .flatten()
        .copied()
        .map(rewrite)
        .collect();
    super::super::lambda_seed::seed_patterns(
        lookup,
        arena,
        args,
        &patterns,
        &Default::default(),
        profile.delegate_wrappers,
    );
    yielded.map(rewrite)
}

#[cfg(test)]
#[path = "chain_namespace_root_tests.rs"]
mod tests;
