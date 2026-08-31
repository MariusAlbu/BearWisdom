// =============================================================================
// engine/type_slots — identity-scoped return/field type reads
//
// A resolved member is one specific row, but an overload set or a merged
// declaration records its type on ONE row of the same-qname set — often not
// the row a member lookup returned. These probes read the symbol's own id
// slot first, then the id slots of same-FILE declarations sharing its qname.
// The global qname slot is never consulted: it is first-writer-wins across
// packages, so reading it after an id miss can only answer with an unrelated
// same-named declaration's type.
// =============================================================================

use crate::type_checker::core::types::TypeId;

use super::contract::{Symbol, SymbolLookup};

/// The return type of `sym` by identity — own id slot, then same-file
/// same-qname sibling rows.
pub(super) fn return_type_by_identity(lookup: &dyn SymbolLookup, sym: &Symbol) -> Option<TypeId> {
    if let Some(id) = lookup.return_type_id_of(sym.id) {
        return Some(id);
    }
    for sib in lookup.all_by_qualified_name(&sym.qualified_name).iter() {
        if sib.id != sym.id && sib.file_path == sym.file_path {
            if let Some(id) = lookup.return_type_id_of(sib.id) {
                return Some(id);
            }
        }
    }
    None
}

/// The field/declared type of `sym` by identity — own id slot, then same-file
/// same-qname sibling rows.
pub(super) fn field_type_by_identity(lookup: &dyn SymbolLookup, sym: &Symbol) -> Option<TypeId> {
    if let Some(id) = lookup.field_type_id_of(sym.id) {
        return Some(id);
    }
    for sib in lookup.all_by_qualified_name(&sym.qualified_name).iter() {
        if sib.id != sym.id && sib.file_path == sym.file_path {
            if let Some(id) = lookup.field_type_id_of(sib.id) {
                return Some(id);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "type_slots_tests.rs"]
mod tests;
