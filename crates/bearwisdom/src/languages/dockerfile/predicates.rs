// =============================================================================
// dockerfile/predicates.rs — Dockerfile builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls | EdgeKind::TypeRef => matches!(sym_kind, "class" | "variable"),
        _ => true,
    }
}

