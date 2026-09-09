//! Qualified calls anchor the value chain using explicit source type identities.
use super::*;

pub(super) fn anchor(
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
    arena: &TypeArena,
    profile: &LanguageProfile,
) -> Option<Result<namespace_root::Anchor, Option<Cause>>> {
    let reference = ref_ctx.extracted_ref;
    let chain = reference.chain.as_ref()?;
    let segment = chain.segments.get(1)?;
    if !segment.is_call || !lookup.qualified_call_site(segment.byte_offset) {
        return None;
    }
    let terminal = chain.segments.len() == 2;
    // The selector's operands were attested together by CST stamping. The
    // outer reference still carries legacy display-oriented extractor args.
    let Some(args) = super::super::arg_types::at(lookup, segment.byte_offset, &segment.call_args)
    else {
        return Some(Err(None));
    };
    let actual = resolve_arg_types(lookup, arena, args);
    let explicit = match lookup.member_type_arguments(segment.byte_offset) {
        Some(arguments) => arguments,
        None if segment.type_args.is_empty() && segment.type_arg_ids.is_empty() => &[],
        None => return Some(Err(None)),
    };
    let selected = lookup.qualified_call(segment.byte_offset, &actual, explicit)?;
    Some(selected.map_err(|_| None).and_then(|call| {
        let ordinary = args.get(call.receiver_arguments..).ok_or(None)?;
        super::super::lambda_seed::seed_patterns(
            lookup,
            arena,
            ordinary,
            &call.parameters,
            &Default::default(),
            profile.delegate_wrappers,
        );
        if terminal {
            Ok(namespace_root::Anchor::Terminal(SymbolInfo {
                target_symbol_id: call.declaration,
                confidence: RESOLVED_CONFIDENCE,
                strategy: "qualified_trait",
                resolved_yield_type: call.return_type,
                flow_emit: None,
            }))
        } else {
            call.return_type
                .map(|ty| namespace_root::Anchor::Receiver(Receiver::untyped(ty), 2))
                .ok_or(None)
        }
    }))
}

#[cfg(test)]
#[path = "chain_qualified_call_tests.rs"]
mod tests;
