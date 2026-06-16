// =============================================================================
// type_checker/core/symbol_view.rs — unified per-symbol read facade
//
// One borrow over a symbol's identity (`SymbolInfo`) and its type bundle
// (`SymbolTypeMap`, keyed by symbol id). Consumers ask `view.param_types()`
// instead of threading both structures and keying each by hand. Structural
// containment is an id edge consulted through `SymbolLookup`, not carried here.
//
// The type accessors are an honest tri-state: `None` means "no
// SymbolTypeData record for this id" (the symbol's types were never recorded
// or aren't hydrated yet), distinct from `Some(&[])` which means "recorded
// with zero parameters". A caller that needs to tell "no info" from "zero
// args" must branch on the `Option`, never collapse it.
// =============================================================================

use crate::indexer::resolve::legacy::SymbolInfo;
use crate::type_checker::core::symbol_types::{SymbolTypeData, SymbolTypeMap};
use crate::type_checker::core::types::{GenericParamId, TypeId};

/// Read-only view over one symbol's identity, type bundle, and containment.
/// Cheap to build (a struct of borrows); construct per query.
pub struct SymbolView<'a> {
    pub info: &'a SymbolInfo,
    types: &'a SymbolTypeMap,
}

impl<'a> SymbolView<'a> {
    /// Construct a view over a symbol's identity and type bundle.
    pub fn new(info: &'a SymbolInfo, types: &'a SymbolTypeMap) -> Self {
        Self { info, types }
    }

    /// Positional parameter types. `None` when no `SymbolTypeData` record
    /// exists for this symbol id (no type info, or not yet hydrated);
    /// `Some(&[])` when a record exists but declares zero parameters.
    pub fn param_types(&self) -> Option<&'a [TypeId]> {
        self.types
            .get(self.info.id)
            .map(|d| d.param_types.as_slice())
    }

    /// Return type. `None` when no record exists OR the record carries no
    /// return type — both collapse to "no return type known" here, which is
    /// what the only meaningful distinction (present/absent) needs.
    pub fn return_type(&self) -> Option<TypeId> {
        self.types.get(self.info.id).and_then(|d| d.return_type)
    }

    /// Declared (annotated/inferred) type for a value-bearing symbol. `None`
    /// when no record exists or the record carries no declared type.
    pub fn declared_type(&self) -> Option<TypeId> {
        self.types.get(self.info.id).and_then(|d| d.declared_type)
    }

    /// Generic parameter slots bound by this symbol. Empty slice when none
    /// are recorded; the empty-vs-absent distinction is not meaningful for
    /// generics, so this collapses both to an empty slice.
    pub fn generic_params(&self) -> &'a [GenericParamId] {
        self.types
            .get(self.info.id)
            .map(|d| d.generic_params.as_slice())
            .unwrap_or(&[])
    }

    /// The whole `SymbolTypeData` record this view is over, or `None` when no
    /// record exists for this id. The per-field accessors above collapse "no
    /// record" into their `None`/empty result; this accessor is the one that
    /// preserves the record-existence distinction, for a caller that needs it
    /// or that reads several fields together under a single borrow. Prefer the
    /// per-field accessors otherwise.
    pub fn type_data(&self) -> Option<&'a SymbolTypeData> {
        self.types.get(self.info.id)
    }
}

#[cfg(test)]
#[path = "symbol_view_tests.rs"]
mod tests;
