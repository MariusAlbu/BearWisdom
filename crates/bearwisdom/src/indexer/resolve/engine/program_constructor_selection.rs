//! Constructor applicability retains source origins, including absent implicit origins.
use super::*;
use crate::indexer::resolve::engine::contract::flow_cache::CallSignatureOrigin;
use crate::type_checker::core::types::TypeId;

pub(super) struct Candidate {
    pub owner: i64,
    pub origin: Option<CallSignatureOrigin>,
    pub signature: source_signatures::Bound,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Call {
    pub origins: Vec<Option<CallSignatureOrigin>>,
    pub selected: Option<usize>,
    pub return_type: TypeId,
}

pub(super) fn select(
    lookup: &Lookup,
    arena: &TypeArena,
    candidates: Vec<Candidate>,
    arguments: &[TypeId],
    types: &[TypeId],
) -> Option<Call> {
    use super::overload_calls::ordering;
    let facts = candidates
        .iter()
        .map(|candidate| {
            let origin = candidate.origin.as_ref()?;
            Some(ordering::Fact {
                owner: candidate.owner,
                source: origin.source,
                span: origin.span,
                order: candidate.signature.syntax.ordering?,
            })
        })
        .collect::<Option<Vec<_>>>();
    let order = facts.and_then(|facts| ordering::source_candidates(lookup, &facts));
    let signatures = candidates
        .iter()
        .map(|candidate| &candidate.signature)
        .collect::<Vec<_>>();
    let selection = super::call_selection::select(
        &super::merge_proof::types::Relation { lookup, arena },
        &signatures,
        order.as_deref(),
        arguments,
        types,
        &|_| false,
        &|_, _| None,
        true,
    )?;
    Some(Call {
        origins: candidates
            .into_iter()
            .map(|candidate| candidate.origin)
            .collect(),
        selected: selection.selected,
        return_type: selection.applied.result,
    })
}
