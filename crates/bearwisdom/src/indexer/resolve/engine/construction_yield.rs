// =============================================================================
// engine/construction_yield — the type a construction call yields
//
// Construction yields the declaration being constructed. That fact is
// language-independent semantics, so it lives in the generic engine: the index
// records the yield for the declaration symbol, and for the constructor symbol
// through its structural parent link — a constructor's signature names no
// return, so the signature-inference chain has nothing to read for it.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::{TypeArena, TypeId};
use crate::types::{ExtractedSymbol, SymbolKind};

use super::contract::TypeInfo;
use super::kinds::is_type_kind;

#[cfg(test)]
#[path = "construction_yield_tests.rs"]
mod tests;

/// The type a construction call on `decl` yields — the declaration itself.
pub(super) fn declaration_yield(arena: &TypeArena, decl: &ExtractedSymbol) -> TypeId {
    arena.class(&decl.qualified_name)
}

/// The declaration a construction call on `sym` yields: a type declaration is
/// its own yield; a constructor's is the declaration its structural parent link
/// names. `None` for any other kind, for a constructor with no structural
/// parent, and for a parent that is not a type declaration — such a constructor
/// names no declaration, and deriving one from the qname spelling would bind by
/// string shape instead of by declaration.
pub(super) fn constructed_declaration<'a>(
    sym: &'a ExtractedSymbol,
    file_symbols: &'a [ExtractedSymbol],
) -> Option<&'a ExtractedSymbol> {
    if is_type_kind(sym.kind.as_str()) {
        return Some(sym);
    }
    if sym.kind != SymbolKind::Constructor {
        return None;
    }
    file_symbols
        .get(sym.parent_index?)
        .filter(|decl| is_type_kind(decl.kind.as_str()))
}

/// First-writer-wins write of `rid` into the qname-keyed and id-keyed return
/// slots. The id slot keeps declarations that share a qname across packages
/// distinct; the qname slot stays for readers that have only a name.
pub(super) fn record_return_type(
    qname: &str,
    sym_id: Option<i64>,
    rid: TypeId,
    by_qname: &mut FxHashMap<String, TypeInfo>,
    by_id: &mut FxHashMap<i64, TypeInfo>,
) {
    let ti = by_qname.entry(qname.to_string()).or_default();
    if ti.return_type_id.is_none() {
        ti.return_type_id = Some(rid);
    }
    if let Some(id) = sym_id {
        let tid = by_id.entry(id).or_default();
        if tid.return_type_id.is_none() {
            tid.return_type_id = Some(rid);
        }
    }
}
