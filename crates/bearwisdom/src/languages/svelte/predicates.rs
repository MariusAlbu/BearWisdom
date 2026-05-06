// =============================================================================
// svelte/builtins.rs — Svelte builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// Delegates to the TypeScript rules — Svelte's script blocks are TypeScript.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    crate::languages::typescript::predicates::kind_compatible(edge_kind, sym_kind)
}
