// =============================================================================
// bicep/predicates.rs — Bicep edge/symbol kind compatibility
//
// Builtin classification has moved out of this module. Bicep system-namespace
// functions, decorators, and ARM resource API methods are discovered at index
// time by the `bicep-runtime` ecosystem (see
// `crates/bearwisdom/src/ecosystem/bicep_runtime.rs`) — names come from a
// local Azure/bicep source clone. When no clone is present the names are
// genuinely unindexable from this machine and refs to them stay unresolved.
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "variable" | "function"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}
