// =============================================================================
// engine/enclosing — enclosing-scope derivation from parent_index ancestry
//
// Builds the enclosing-type / enclosing-namespace maps a `this`/`self` root
// and the enclosing-member rule consult: qname-keyed views plus the identity
// twin (source row id → enclosing type's row id).
// =============================================================================

use rustc_hash::{FxHashMap, FxHashSet};

use crate::indexer::write::SymbolIds;
use crate::types::{ExtractedSymbol, ParsedFile};

use super::contract::is_type_like_kind as is_type_like;

/// Derive every symbol's nearest enclosing type / namespace for one file and
/// insert into the given maps.
pub(super) fn build_enclosing_maps(
    pf: &ParsedFile,
    symbol_ids: &SymbolIds,
    enclosing_type: &mut FxHashMap<String, String>,
    enclosing_type_by_id: &mut FxHashMap<i64, i64>,
    enclosing_namespace: &mut FxHashMap<String, String>,
) {
    for (sym_i, sym) in pf.symbols.iter().enumerate() {
        let (found_type, found_ns) = enclosing_chain(&pf.symbols, sym.parent_index);
        if let Some(t_idx) = found_type {
            let t = &pf.symbols[t_idx];
            enclosing_type.insert(sym.qualified_name.clone(), t.qualified_name.clone());
            // Identity twin: source row id → enclosing type's row id.
            if let (Some(src_id), Some(t_id)) = (
                symbol_ids.id_of(&pf.path, sym_i, &sym.qualified_name),
                symbol_ids.id_of(&pf.path, t_idx, &t.qualified_name),
            ) {
                enclosing_type_by_id.insert(src_id, t_id);
            }
        }
        if let Some(n_idx) = found_ns {
            enclosing_namespace.insert(
                sym.qualified_name.clone(),
                pf.symbols[n_idx].qualified_name.clone(),
            );
        }
    }
}

/// Walk a symbol's `parent_index` ancestry to the nearest enclosing type-like
/// symbol and the nearest enclosing namespace/module, returning their qualified
/// names as `(enclosing_type, enclosing_namespace)`. Stops as soon as both are
/// found.
///
/// Cycle-guarded: a `parent_index` chain that revisits an index — a self-parent
/// (`parent_index` == the symbol's own slot) or a longer loop, which a
/// name-based symbol merge can produce when a same-named parent and descendant
/// collapse onto one slot — terminates at the first repeat. Without the guard a
/// self-parented symbol that never reaches both a type AND a namespace ancestor
/// spins forever.
fn enclosing_chain(
    symbols: &[ExtractedSymbol],
    start: Option<usize>,
) -> (Option<usize>, Option<usize>) {
    let mut cursor = start;
    let mut found_type: Option<usize> = None;
    let mut found_ns: Option<usize> = None;
    let mut visited: FxHashSet<usize> = FxHashSet::default();
    while let Some(idx) = cursor {
        if !visited.insert(idx) {
            break;
        }
        let Some(ancestor) = symbols.get(idx) else {
            break;
        };
        let ancestor_kind = ancestor.kind.as_str();
        if found_type.is_none() && is_type_like(ancestor_kind) {
            found_type = Some(idx);
        }
        if found_ns.is_none() && matches!(ancestor_kind, "namespace" | "module") {
            found_ns = Some(idx);
        }
        if found_type.is_some() && found_ns.is_some() {
            break;
        }
        cursor = ancestor.parent_index;
    }
    (found_type, found_ns)
}

#[cfg(test)]
pub(crate) fn _test_enclosing_chain(
    symbols: &[ExtractedSymbol],
    start: Option<usize>,
) -> (Option<usize>, Option<usize>) {
    enclosing_chain(symbols, start)
}
