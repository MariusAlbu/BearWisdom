// =============================================================================
// gleam/type_checker.rs — Gleam type checker (minimal)
//
// Provides language identity and `kind_compatible` for the registry. This
// language doesn't currently walk member chains via the unified type-checker
// path, so `resolve_chain` inherits the trait's `None` default. PR 8 of
// decision-2026-04-27-e75 — every supported language gets a TypeChecker so
// the engine's checker registry is dense.
// =============================================================================

use crate::type_checker::TypeChecker;
use crate::types::EdgeKind;

pub struct GleamChecker;

impl TypeChecker for GleamChecker {
    fn language_id(&self) -> &str {
        "gleam"
    }

}
