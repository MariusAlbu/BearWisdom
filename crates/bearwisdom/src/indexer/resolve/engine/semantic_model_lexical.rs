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
    crate::tracef!(
        "  LEXICAL '{}' @{} declaration={:?} kind={:?}",
        reference.target_name,
        reference.byte_offset,
        local.declaration,
        local.kind
    );
    let Some(target_symbol_id) = local.declaration else {
        if let Some(info) = bind_import_overload(reference, &local, lookup) {
            return Some(SolveOutcome::Resolved(info));
        }
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

/// Bind an import-attributed type reference — the `import { x }` site itself,
/// or a use site's type-position twin that names its import — to the
/// declaration the file's lexical binding resolved that identifier to. `None`
/// when the reference carries no import attribution, has no lexical binding,
/// or the binding resolved to no single declaration: the ladder's own import
/// evidence runs.
pub(super) fn bind_lexical_import_binding(
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<SolveOutcome> {
    let reference = ref_ctx.extracted_ref;
    let import_attributed = reference.is_import_binding
        || (reference.kind == EdgeKind::TypeRef && reference.module.is_some());
    if !import_attributed {
        return None;
    }
    let local = lookup.local_reference(reference.byte_offset)?;
    let target_symbol_id = local.declaration?;
    crate::tracef!(
        "  LEXICAL import '{}' @{} declaration={}",
        reference.target_name,
        reference.byte_offset,
        target_symbol_id
    );
    Some(SolveOutcome::Resolved(SymbolInfo {
        target_symbol_id,
        confidence: super::super::contract::RESOLVED_CONFIDENCE,
        strategy: "lexical_binding",
        resolved_yield_type: None,
        flow_emit: None,
    }))
}

/// A binding whose import names an overload group has no single declaration;
/// the call's typed arguments select one signature, and that signature's row
/// is the target. Anything short of a selection leaves the miss diagnosed.
fn bind_import_overload(
    reference: &crate::types::ExtractedRef,
    local: &super::super::contract::flow_cache::LocalReference,
    lookup: &dyn SymbolLookup,
) -> Option<SymbolInfo> {
    let arena = lookup.type_arena()?;
    let args = super::super::arg_types::at(lookup, reference.byte_offset, &reference.call_args)?;
    let actual = super::super::arg_types::resolve_arg_types(lookup, arena, args);
    let call = lookup
        .overloaded_import_call(reference.byte_offset, &actual, &local.type_args)?
        .ok()?;
    let origin = call.origins.get(call.selected)?;
    Some(SymbolInfo {
        target_symbol_id: origin.declaration?,
        confidence: super::super::contract::RESOLVED_CONFIDENCE,
        strategy: "lexical_overload",
        resolved_yield_type: Some(call.return_type),
        flow_emit: None,
    })
}
