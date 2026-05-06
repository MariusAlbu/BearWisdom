// =============================================================================
// vue/builtins.rs — Vue builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// Delegates to TypeScript rules — Vue script blocks are TypeScript/JavaScript.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    crate::languages::typescript::predicates::kind_compatible(edge_kind, sym_kind)
}

