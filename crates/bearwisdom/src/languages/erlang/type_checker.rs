// =============================================================================
// erlang/type_checker.rs — Erlang type checker (minimal)
//
// Provides language identity and `kind_compatible` for the registry. This
// language doesn't currently walk member chains via the unified type-checker
// path, so `resolve_chain` inherits the trait's `None` default. PR 8 of
// decision-2026-04-27-e75 — every supported language gets a TypeChecker so
// the engine's checker registry is dense.
// =============================================================================

use super::predicates;
use crate::type_checker::TypeChecker;
use crate::types::EdgeKind;

pub struct ErlangChecker;

impl TypeChecker for ErlangChecker {
    fn language_id(&self) -> &str {
        "erlang"
    }

    fn kind_compatible(&self, edge_kind: EdgeKind, sym_kind: &str) -> bool {
        predicates::kind_compatible(edge_kind, sym_kind)
    }
}
