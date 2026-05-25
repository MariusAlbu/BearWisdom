// =============================================================================
// nunjucks/predicates.rs — kind-compatibility predicate.
// =============================================================================

use crate::types::EdgeKind;

pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method" | "macro" | "class"),
        EdgeKind::TypeRef => matches!(sym_kind, "class" | "interface" | "type_alias"),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}
