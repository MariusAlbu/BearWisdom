//! Shared ordered dispatch; result agreement alone does not prove a signature.
use super::{
    call_arguments::{self, Applied},
    merge_proof::types::{ArgumentRelation, Relation},
    source_signatures::Bound,
};
use crate::type_checker::core::types::TypeId;

pub(super) struct Selection {
    pub selected: Option<usize>,
    pub applied: Applied,
}

pub(super) fn select(
    relation: &Relation,
    candidates: &[&Bound],
    order: Option<&[usize]>,
    actual: &[TypeId],
    explicit: &[TypeId],
    deferred: &dyn Fn(usize) -> bool,
    callback: &dyn Fn(usize, TypeId) -> Option<TypeId>,
    consensus: bool,
) -> Option<Selection> {
    if candidates.is_empty() || candidates.len() > 4096 {
        return None;
    }
    if let Some(order) = order {
        if order.len() != candidates.len() {
            return None;
        }
        let mut seen = rustc_hash::FxHashSet::default();
        if order
            .iter()
            .any(|&i| i >= candidates.len() || !seen.insert(i))
        {
            return None;
        }
        use ArgumentRelation::{Assignable, Subtype};
        let phases: &[_] = if candidates.len() > 1 {
            &[Subtype, Assignable]
        } else {
            &[Assignable]
        };
        for &phase in phases {
            for &index in order {
                // Unknown earlier evidence blocks the phase. A selected
                // signature does not depend on the unsupported later suffix.
                if let Some(applied) = call_arguments::contextual_in_phase(
                    relation,
                    candidates[index],
                    actual,
                    explicit,
                    deferred,
                    callback,
                    phase,
                )? {
                    return Some(Selection {
                        selected: Some(index),
                        applied,
                    });
                }
            }
        }
        return None;
    }
    let mut result: Option<Selection> = None;
    for (index, signature) in candidates.iter().enumerate() {
        if let Some(applied) =
            call_arguments::contextual(relation, signature, actual, explicit, deferred, callback)?
        {
            if let Some(prior) = &mut result {
                if !consensus || prior.applied.result != applied.result {
                    return None;
                }
                prior.selected = None;
                // Consensus proves only the result, not selected parameter types.
                prior.applied.parameters.clear();
            } else {
                result = Some(Selection {
                    selected: Some(index),
                    applied,
                });
            }
        }
    }
    result
}
