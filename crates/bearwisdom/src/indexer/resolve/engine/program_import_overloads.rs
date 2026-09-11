//! An imported overload group selects one signature by argument evidence in
//! declaration order. The group binds by row identity or not at all.
use super::super::{call_selection, merge_proof::types::Relation, Lookup};
use crate::indexer::resolve::engine::{
    contract::flow_cache::{CallSignatureOrigin, OverloadCall},
    contract::SymbolLookup,
    module_paths,
    program_graph::SourceInstanceId,
    program_types::source_signatures::{Bound, Input},
};
use crate::type_checker::core::types::TypeId;

/// `None`: `rows` is not a group this view attests — fewer than two rows, a
/// row outside the calling source's program, or a row without a source
/// signature. `Some(Err)`: an attested group no signature of which accepts
/// the call.
pub(in crate::indexer::resolve::engine) fn select(
    lookup: &Lookup,
    rows: &[i64],
    actual: &[TypeId],
    explicit: &[TypeId],
) -> Option<Result<OverloadCall, ()>> {
    if rows.len() < 2 {
        return None;
    }
    let arena = lookup.type_arena()?;
    let candidates = attest(lookup, rows)?;
    let order = declaration_order(&candidates);
    let bounds: Vec<&Bound> = candidates.iter().map(|c| c.bound).collect();
    let relation = Relation { lookup, arena };
    let Some(selection) = call_selection::select(
        &relation,
        &bounds,
        Some(&order),
        actual,
        explicit,
        &|_| false,
        &|_, _| None,
        false,
    ) else {
        return Some(Err(()));
    };
    let selected = selection.selected?;
    Some(Ok(OverloadCall {
        origins: candidates
            .iter()
            .map(|c| CallSignatureOrigin {
                source: c.source,
                span: c.input.id.0,
                declaration: Some(c.row),
            })
            .collect(),
        selected,
        return_type: selection.applied.result,
        parameters: selection.applied.parameters,
    }))
}

struct Candidate<'a> {
    row: i64,
    source: SourceInstanceId,
    input: &'a Input,
    bound: &'a Bound,
}

/// Every row must be a signature-bearing declaration whose file belongs to
/// the calling source's own program view; the bound signature is that view's.
fn attest<'a>(lookup: &'a Lookup<'a>, rows: &[i64]) -> Option<Vec<Candidate<'a>>> {
    let mut candidates = Vec::with_capacity(rows.len());
    for &row in rows {
        let symbol = lookup.symbol_by_id(row)?;
        let path = module_paths::normalize(&symbol.file_path);
        let declaring = lookup.tree.program_lookup(&path)?;
        if !std::ptr::eq(declaring.view, lookup.view) {
            return None;
        }
        let source = declaring.source?;
        let types = lookup
            .view
            .modules
            .inputs
            .get(&path)?
            .globals
            .as_ref()?
            .types
            .as_ref()?;
        let input = types
            .source_signatures
            .iter()
            .find(|s| s.declaration == Some(row))?;
        candidates.push(Candidate {
            row,
            source: source.identity?,
            input,
            bound: source.signatures.get(&input.id)?,
        });
    }
    Some(candidates)
}

/// Candidate indices in source order of their signatures.
fn declaration_order(candidates: &[Candidate<'_>]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|&i| candidates[i].input.id.0.start);
    order
}

#[cfg(test)]
#[path = "program_import_overloads_tests.rs"]
mod tests;
