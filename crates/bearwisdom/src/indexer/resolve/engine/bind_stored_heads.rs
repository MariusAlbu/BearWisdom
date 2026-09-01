// =============================================================================
// engine/bind_stored_heads — bind stored type slots to their declarations
//
// Stored field/return types are interned from strings at extract/ingest time,
// so their heads are name-addressed `Class` values and every member hop's
// yield re-runs string recovery. This pass rewrites each slot whose head
// names exactly ONE type-like declaration to the bound `Decl` form, so
// yields carry identity through the walk. Ambiguous and unknown heads keep
// their `Class` form — read-time recovery (package preference, import
// ranking) still owns those, by design.
// =============================================================================

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::{TypeArena, TypeId};

use super::contract::{SymbolLookup, TypeInfo};

/// Rewrite the id-keyed type slots in place. Memoized per source TypeId —
/// many symbols share one stored type — and idempotent (bound heads bind to
/// themselves via the early return in `bind_head_unique`).
pub(super) fn apply(
    arena: &TypeArena,
    lookup: &dyn SymbolLookup,
    type_info_by_id: &mut FxHashMap<i64, TypeInfo>,
) {
    let mut memo: FxHashMap<TypeId, Option<TypeId>> = FxHashMap::default();
    let mut bind = |ty: TypeId| -> Option<TypeId> {
        *memo
            .entry(ty)
            .or_insert_with(|| super::head_decl::bind_head_unique(arena, lookup, ty))
    };
    for info in type_info_by_id.values_mut() {
        if let Some(rt) = info.return_type_id {
            if let Some(bound) = bind(rt) {
                info.return_type_id = Some(bound);
            }
        }
        if let Some(ft) = info.field_type_id {
            if let Some(bound) = bind(ft) {
                info.field_type_id = Some(bound);
            }
        }
    }
}

#[cfg(test)]
#[path = "bind_stored_heads_tests.rs"]
mod tests;
