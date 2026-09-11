// =============================================================================
// semantic_model_lexical — bare calls answered by the file's lexical binding
// =============================================================================

use super::*;

/// Bind a chain-less call or construction through the file's lexical binding
/// at the reference site. `Some` is the model's answer; `None` hands the ref
/// to the rule ladder.
///
/// A binding with a persisted declaration resolves to it. A binding without
/// one (an import whose target has no persisted row, or a known local without
/// a row) is a diagnosed miss: the ladder never runs, because a name-only
/// match is not identity, and the cause names what the file did declare.
pub(super) fn bind_lexical_call(
    ref_ctx: &RefContext,
    file_ctx: &FileContext,
    lookup: &dyn SymbolLookup,
) -> Option<SolveOutcome> {
    let reference = ref_ctx.extracted_ref;
    if !matches!(reference.kind, EdgeKind::Calls | EdgeKind::Instantiates)
        || reference
            .chain
            .as_ref()
            .is_some_and(|c| c.segments.len() >= 2)
    {
        return None;
    }
    let local = lookup.local_reference(reference.byte_offset)?;
    let Some(target_symbol_id) = local.declaration else {
        return Some(SolveOutcome::Unresolved(Some(
            super::super::unbound_cause::classify_unbound_root(
                &reference.target_name,
                &ref_ctx.scope_chain,
                file_ctx,
                lookup,
                ref_ctx.file_package_id,
            ),
        )));
    };
    let resolved_yield_type = lookup.type_arena().and_then(|arena| {
        super::super::lexical_value::yield_type(
            &local,
            lookup,
            arena,
            reference.kind == EdgeKind::Instantiates,
        )
    });
    Some(SolveOutcome::Resolved(SymbolInfo {
        target_symbol_id,
        confidence: super::super::contract::RESOLVED_CONFIDENCE,
        strategy: "lexical_binding",
        resolved_yield_type,
        flow_emit: None,
    }))
}
