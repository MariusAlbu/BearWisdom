// =============================================================================
// indexer/contract_filter — reduce a parsed file to its declaration contract
//
// External supply is consumed declaration-first: type/member rows, signatures,
// type mentions, inheritance, imports/re-exports, alias shapes. Function-body
// internals — locals, nested closures, body call/read/write refs — are never
// readable through the resolver's external surface, so a supply artifact
// stores only the contract. The filter is language-agnostic: body membership
// is derived purely from the symbol parent chain and kind.
// =============================================================================

use crate::types::{EdgeKind, ParsedFile, SymbolKind};

/// Whether a symbol kind introduces a body whose descendants are
/// implementation detail rather than contract surface.
fn is_callable(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor
    )
}

/// Ref kinds that describe a declaration's contract: what it extends, what
/// types its signature mentions, and what it imports or re-exports. Body
/// `Calls`/`Instantiates`/`Reads`/`Writes` never produce external-readable
/// data and are dropped.
fn is_contract_ref(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Inherits | EdgeKind::Implements | EdgeKind::TypeRef | EdgeKind::Imports
    )
}

/// Reduce `pf` in place to its contract. Symbols beneath a callable are
/// dropped, except a callable's OWN parameters (they carry the signature) —
/// and only when that callable is itself contract-level (a nested closure's
/// parameters go with the closure). Refs keep contract kinds from surviving
/// sources only; parallel per-symbol/per-ref vectors and index links are
/// remapped in step. Body-level extras (flow metadata, routes, db sets) are
/// cleared.
pub fn reduce_to_contract(pf: &mut ParsedFile) {
    // Out-of-line bodies can precede their nominal owners in source order.
    let n = pf.symbols.len();
    let under_callable = callable_ancestry(&pf.symbols);
    let keep: Vec<bool> = (0..n)
        .map(|i| {
            if !under_callable[i] {
                return true;
            }
            // A contract-level callable's own parameters survive.
            pf.symbols[i].kind == SymbolKind::Parameter
                && pf.symbols[i].parent_index.is_some_and(|p| {
                    pf.symbols
                        .get(p)
                        .is_some_and(|s| is_callable(s.kind) && !under_callable[p])
                })
        })
        .collect();

    // Old index → new index for survivors.
    let mut remap: Vec<Option<usize>> = Vec::with_capacity(n);
    let mut next = 0usize;
    for &k in &keep {
        remap.push(k.then(|| {
            let v = next;
            next += 1;
            v
        }));
    }

    let take = |i: usize| keep[i];
    let mut idx = 0usize;
    pf.symbols.retain(|_| {
        let k = take(idx);
        idx += 1;
        k
    });
    for s in pf.symbols.iter_mut() {
        s.parent_index = s.parent_index.and_then(|p| remap.get(p).copied().flatten());
    }
    retain_parallel(&mut pf.symbol_origin_languages, &keep);
    retain_parallel(&mut pf.symbol_from_snippet, &keep);

    let mut kept_ref = vec![false; pf.refs.len()];
    for (i, r) in pf.refs.iter().enumerate() {
        kept_ref[i] = is_contract_ref(r.kind)
            && remap
                .get(r.source_symbol_index)
                .copied()
                .flatten()
                .is_some();
    }
    let mut idx = 0usize;
    pf.refs.retain(|_| {
        let k = kept_ref[idx];
        idx += 1;
        k
    });
    for r in pf.refs.iter_mut() {
        if let Some(new) = remap.get(r.source_symbol_index).copied().flatten() {
            r.source_symbol_index = new;
        }
    }
    retain_parallel(&mut pf.ref_origin_languages, &kept_ref);

    // `content` is deliberately untouched: its lifecycle belongs to the
    // caller (plugin cross-file passes CST-walk it; the streaming loop nulls
    // it) — the contract reduction governs symbols and refs only.
    pf.flow = Default::default();
    super::contract_bindings::restore(pf);
    pf.routes.clear();
    pf.db_sets.clear();
}

/// Memoized parent-ID walk: linear work, independent of physical row order.
/// Cyclic/dangling ancestry is not evidence for a public contract declaration.
fn callable_ancestry(symbols: &[crate::types::ExtractedSymbol]) -> Vec<bool> {
    // Unknown / active / outside body / inside body (or invalid ancestry).
    let mut state = vec![0u8; symbols.len()];
    let mut trail = Vec::new();
    for start in 0..symbols.len() {
        if state[start] >= 2 {
            continue;
        }
        let mut current = start;
        let under = loop {
            match state[current] {
                1 | 3 => break true,
                2 => break false,
                _ => {}
            }
            state[current] = 1;
            trail.push(current);
            match symbols[current].parent_index {
                None => break false,
                Some(parent) if parent >= symbols.len() => break true,
                Some(parent) if is_callable(symbols[parent].kind) => break true,
                Some(parent) => current = parent,
            }
        };
        for index in trail.drain(..) {
            state[index] = if under { 3 } else { 2 };
        }
    }
    state.into_iter().map(|s| s == 3).collect()
}

/// Reduce only the REF side of `pf` to contract kinds, keeping every symbol.
/// The safe reduction for error-recovered parses: their symbol parent chains
/// are unreliable (an include fragment's top-level functions parse as nested),
/// but ref kinds are trustworthy — and body call/read/write refs are never
/// consumed through the external surface regardless of parse quality.
pub fn reduce_refs_to_contract(pf: &mut ParsedFile) {
    let kept: Vec<bool> = pf.refs.iter().map(|r| is_contract_ref(r.kind)).collect();
    let mut idx = 0usize;
    pf.refs.retain(|_| {
        let k = kept[idx];
        idx += 1;
        k
    });
    retain_parallel(&mut pf.ref_origin_languages, &kept);
    pf.flow = Default::default();
    super::contract_bindings::restore(pf);
    pf.routes.clear();
    pf.db_sets.clear();
}

/// Retain a vector parallel to a filtered one; an empty vector stays empty
/// (the schema treats empty as all-default).
fn retain_parallel<T>(v: &mut Vec<T>, keep: &[bool]) {
    if v.is_empty() {
        return;
    }
    let mut idx = 0usize;
    v.retain(|_| {
        let k = keep.get(idx).copied().unwrap_or(false);
        idx += 1;
        k
    });
}

#[cfg(test)]
#[path = "contract_filter_tests.rs"]
mod tests;
