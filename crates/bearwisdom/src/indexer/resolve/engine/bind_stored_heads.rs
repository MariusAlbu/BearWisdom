// =============================================================================
// engine/bind_stored_heads — bind stored type slots to their declarations
//
// Stored field/return types are interned from strings at extract/ingest time,
// so their heads are name-addressed `Class` values and every member hop's
// yield re-runs string recovery. Slot reads route through `bound`, which
// rewrites a head naming exactly ONE type-like declaration to the bound
// `Decl` form, so yields carry identity through the walk. Ambiguous and
// unknown heads keep their `Class` form — read-time recovery (package
// preference, import ranking) still owns those, by design.
//
// Binding is computed on first read and memoized for the compilation's
// lifetime: the name tables are complete before any slot is read, so a
// result never goes stale. Two threads may compute the same bind
// concurrently; interning is deterministic, so both produce the same
// TypeId and either insert wins.
// =============================================================================

use std::sync::RwLock;

use rustc_hash::FxHashMap;

use crate::type_checker::core::types::{TypeArena, TypeId};

use super::contract::SymbolLookup;

/// Per-compilation memo: stored-slot TypeId → its bound rewrite (`None` = the
/// head is bare, ambiguous, unknown, or already bound — keep the stored id).
#[derive(Debug, Default)]
pub(super) struct HeadBindMemo(RwLock<FxHashMap<TypeId, Option<TypeId>>>);

impl HeadBindMemo {
    /// The bound form of stored slot type `ty`: its `Decl`-headed rewrite when
    /// the head names exactly one type-like declaration, `ty` itself otherwise.
    pub(super) fn bound(&self, arena: &TypeArena, lookup: &dyn SymbolLookup, ty: TypeId) -> TypeId {
        if let Some(&cached) = self.0.read().expect("head-bind memo poisoned").get(&ty) {
            return cached.unwrap_or(ty);
        }
        // Computed outside the lock: `bind_head_unique` only reads name tables
        // and interns (deterministic), so concurrent duplicates are identical.
        let bound = super::head_decl::bind_head_unique(arena, lookup, ty);
        self.0.write().expect("head-bind memo poisoned").insert(ty, bound);
        bound.unwrap_or(ty)
    }
}

#[cfg(test)]
#[path = "bind_stored_heads_tests.rs"]
mod tests;
