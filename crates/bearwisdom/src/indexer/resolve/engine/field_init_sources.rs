// =============================================================================
// engine/field_init_sources.rs — which ref initializes which field declaration
//
// A field's initializer reaches the typing pass from one of three places, and
// they disagree about how precisely they name their field. This module ranks
// them so the typing pass reads one map.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::types::{EdgeKind, ExtractedRef, ParsedFile, SymbolKind};

/// Every (field symbol index, initializer ref) pair one file offers, in
/// increasing precision: the lexical declaration slots, the leftmost-call
/// attribution for files with no lexical graph, then the member-target
/// assignments — which name their field exactly and override both.
pub(super) fn field_initializers(pf: &ParsedFile) -> FxHashMap<usize, &ExtractedRef> {
    let mut map: FxHashMap<usize, &ExtractedRef> = lexical_call_initializers(pf)
        .unwrap_or_default()
        .into_iter()
        .collect();
    if pf.flow.lexical.is_none() {
        attribute_leftmost_calls(pf, &mut map);
    }
    for (&ref_idx, &member_idx) in &pf.flow.flow_member_init {
        // Both indices address this file's own vectors; a consumer indexes
        // `symbols` directly, so an out-of-range pair is dropped here.
        let Some(r) = pf.refs.get(ref_idx) else {
            continue;
        };
        if member_idx < pf.symbols.len()
            && matches!(r.kind, EdgeKind::Calls | EdgeKind::Instantiates)
        {
            map.insert(member_idx, r);
        }
    }
    map
}

/// The syntax-owned initializer can be attributed to an enclosing function by
/// the legacy call extractor. Join its physical occurrence to the captured
/// declaration slot, never recover ownership from the call's source symbol.
pub(super) fn lexical_call_initializers(pf: &ParsedFile) -> Option<Vec<(usize, &ExtractedRef)>> {
    let graph = pf.flow.lexical.as_ref()?;
    let mut calls = FxHashMap::default();
    for r in &pf.refs {
        if !matches!(r.kind, EdgeKind::Calls | EdgeKind::Instantiates) {
            continue;
        }
        let selector = r
            .chain
            .as_ref()
            .and_then(|c| c.segments.last())
            .map(|s| s.byte_offset)
            .unwrap_or(r.byte_offset);
        calls.entry((r.byte_offset, selector)).or_insert(r);
    }
    Some(
        graph
            .types
            .call_initializers
            .iter()
            .filter_map(|(&slot, address)| {
                let field = pf.symbols.get(slot)?;
                if !matches!(
                    field.kind,
                    SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable
                ) {
                    return None;
                }
                Some((slot, *calls.get(address)?))
            })
            .collect(),
    )
}

/// Attribution for a file with no lexical graph: a single-segment call whose
/// own source symbol IS a field declaration initializes that field. The
/// earliest occurrence wins — a later reassignment is not the declaration's
/// initializer.
fn attribute_leftmost_calls<'a>(pf: &'a ParsedFile, map: &mut FxHashMap<usize, &'a ExtractedRef>) {
    for r in &pf.refs {
        if !matches!(r.kind, EdgeKind::Calls | EdgeKind::Instantiates) {
            continue;
        }
        if r.chain.as_ref().is_some_and(|c| c.segments.len() > 1) {
            continue;
        }
        let Some(sym) = pf.symbols.get(r.source_symbol_index) else {
            continue;
        };
        if !matches!(
            sym.kind,
            SymbolKind::Property | SymbolKind::Field | SymbolKind::Variable
        ) {
            continue;
        }
        map.entry(r.source_symbol_index)
            .and_modify(|cur| {
                if r.byte_offset < cur.byte_offset {
                    *cur = r;
                }
            })
            .or_insert(r);
    }
}

#[cfg(test)]
#[path = "field_init_sources_tests.rs"]
mod tests;
