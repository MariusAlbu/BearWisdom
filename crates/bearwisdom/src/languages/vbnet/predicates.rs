// =============================================================================
// vbnet/predicates.rs — VB.NET kind-compatibility predicate.
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind. VB.NET shares
/// the .NET type system: classes inherit, interfaces are implemented, modules
/// are sealed-static-class containers.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "property" | "delegate" | "event"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "enum_member" | "type_alias" | "delegate"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}
