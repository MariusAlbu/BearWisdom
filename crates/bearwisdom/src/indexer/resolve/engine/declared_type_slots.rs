// =============================================================================
// engine/declared_type_slots — the extractor-set declared type of a value
//
// A value declaration's own type annotation (`const x: T`, a typed field)
// fills the value's field-type slot, once per batch. Two things never write:
// a value whose qname is ALSO a type declaration — the merged
// `declare var Date: DateConstructor` + `interface Date` pair — because the
// qname slot types INSTANCES of the type and the id map cannot tell the two
// same-qname symbols apart; and an annotation the type text parser could not
// read (`Type::Unknown`), because the slot is first-writer-wins and an
// unknown would shadow the type a later, ref-derived pass proves.
// =============================================================================

use super::compilation::Compilation;
use super::contract::is_type_like_kind;
use super::contract::TypeInfo;
use crate::type_checker::core::types::{Type, TypeId};

/// Write each `(qname, id, declared type)` into the compilation's field-type
/// slots under the two rules above.
pub(super) fn write(compilation: &mut Compilation, pending: Vec<(String, i64, TypeId)>) {
    for (qname, id, type_id) in pending {
        if matches!(compilation.arena.get(type_id), Type::Unknown) {
            continue;
        }
        let qname_owned_by_type = compilation
            .by_qname_all
            .get(&qname)
            .is_some_and(|cands| cands.iter().any(|c| is_type_like_kind(&c.kind)));
        if qname_owned_by_type {
            continue;
        }
        compilation
            .type_info
            .entry(qname)
            .or_insert_with(TypeInfo::default)
            .field_type_id = Some(type_id);
        compilation
            .type_info_by_id
            .entry(id)
            .or_insert_with(TypeInfo::default)
            .field_type_id = Some(type_id);
    }
}

#[cfg(test)]
#[path = "declared_type_slots_tests.rs"]
mod tests;
