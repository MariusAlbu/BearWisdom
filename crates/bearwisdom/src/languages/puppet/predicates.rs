// =============================================================================
// puppet/predicates.rs — edge-kind compatibility
// =============================================================================

use crate::types::EdgeKind;

/// Edge-kind / symbol-kind compatibility for Puppet.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "class"),
        EdgeKind::TypeRef | EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        _ => true,
    }
}
