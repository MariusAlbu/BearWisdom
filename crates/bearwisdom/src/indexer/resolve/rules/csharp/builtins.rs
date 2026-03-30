// =============================================================================
// csharp/builtins.rs — C# builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "namespace" | "delegate"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}

/// Fallback for when no ProjectContext is available.
/// Only recognizes the two always-present .NET SDK prefixes.
pub(super) fn is_external_namespace_fallback(ns: &str) -> bool {
    ns.starts_with("System") || ns.starts_with("Microsoft")
}
