// =============================================================================
// python/predicates.rs — edge-kind compatibility + relative-import shape check
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
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

/// A relative import starts with a dot (`./`, `../`) or is an internal module
/// path (no domain-style host segment, not a stdlib name).
///
/// We approximate: if the module path starts with `.` or contains a `/` it's
/// relative/local. Otherwise it might be an installed package.
pub(super) fn is_relative_import(module: &str) -> bool {
    module.starts_with('.') || module.starts_with('/')
}
