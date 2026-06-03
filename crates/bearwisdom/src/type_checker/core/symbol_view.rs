// =============================================================================
// type_checker/core/symbol_view.rs — unified per-symbol read facade
//
// One borrow over a symbol's identity (`SymbolInfo`), its type bundle
// (`SymbolTypeMap`, keyed by symbol id), and its containment chain
// (`ContainingScope`). Consumers ask `view.param_types()` /
// `view.containing_type()` instead of threading three structures and keying
// each by hand.
//
// The type accessors are an honest tri-state: `None` means "no
// SymbolTypeData record for this id" (the symbol's types were never recorded
// or aren't hydrated yet), distinct from `Some(&[])` which means "recorded
// with zero parameters". A caller that needs to tell "no info" from "zero
// args" must branch on the `Option`, never collapse it.
// =============================================================================

use crate::containment::{ContainingScope, FrameKind, ScopeFrame};
use crate::indexer::resolve::engine::SymbolInfo;
use crate::type_checker::core::symbol_types::SymbolTypeMap;
use crate::type_checker::core::types::{GenericParamId, TypeId};

/// Read-only view over one symbol's identity, type bundle, and containment.
/// Cheap to build (a struct of borrows); construct per query.
pub struct SymbolView<'a> {
    pub info: &'a SymbolInfo,
    types: &'a SymbolTypeMap,
    scope: Option<&'a ContainingScope>,
}

impl<'a> SymbolView<'a> {
    /// View with no containment chain. Type accessors work; the containment
    /// accessors return `None`.
    pub fn new(info: &'a SymbolInfo, types: &'a SymbolTypeMap) -> Self {
        Self { info, types, scope: None }
    }

    /// View backed by a containment chain, enabling `containing_type` /
    /// `containing_namespace`.
    pub fn with_scope(
        info: &'a SymbolInfo,
        types: &'a SymbolTypeMap,
        scope: &'a ContainingScope,
    ) -> Self {
        Self { info, types, scope: Some(scope) }
    }

    /// Positional parameter types. `None` when no `SymbolTypeData` record
    /// exists for this symbol id (no type info, or not yet hydrated);
    /// `Some(&[])` when a record exists but declares zero parameters.
    pub fn param_types(&self) -> Option<&[TypeId]> {
        self.types.get(self.info.id).map(|d| d.param_types.as_slice())
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
    pub fn generic_params(&self) -> &[GenericParamId] {
        self.types
            .get(self.info.id)
            .map(|d| d.generic_params.as_slice())
            .unwrap_or(&[])
    }

    /// The nearest enclosing type frame, excluding the symbol itself —
    /// Roslyn's `ContainingType`. `None` when no containment chain was
    /// supplied or no enclosing type exists.
    pub fn containing_type(&self) -> Option<&ScopeFrame> {
        self.scope?.containing_of_kind(FrameKind::is_type)
    }

    /// The nearest enclosing namespace/module frame, excluding the symbol
    /// itself — Roslyn's `ContainingNamespace`. `None` when no containment
    /// chain was supplied or no enclosing namespace exists.
    pub fn containing_namespace(&self) -> Option<&ScopeFrame> {
        self.scope?.containing_of_kind(FrameKind::is_namespace)
    }
}

#[cfg(test)]
#[path = "symbol_view_tests.rs"]
mod tests;
