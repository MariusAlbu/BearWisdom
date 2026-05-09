// =============================================================================
// haskell/predicates.rs — Haskell builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        // Data constructors (`Just`, `Nothing`, `Left`, `Right`) are
        // first-class values applied like functions — `Just x`, `Right 42`.
        // The extractor classifies them as `enum_member`, so accept that
        // shape for Calls. `variable` covers operator bindings emitted as
        // top-level values (`(<>) = ...`).
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class" | "enum_member" | "variable"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

