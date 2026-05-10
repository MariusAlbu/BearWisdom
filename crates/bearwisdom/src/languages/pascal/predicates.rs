// =============================================================================
// pascal/predicates.rs — Pascal/Delphi builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// The Pascal extractor emits `Calls` for all reference sites — both actual
/// procedure/function calls and typeref nodes (type annotations, variable
/// declarations, etc.) — so `Calls` must accept any addressable symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        // Accept all symbol kinds: the extractor uses Calls for both call
        // expressions and typeref nodes, covering functions, types, enums,
        // records, variables, and properties.
        EdgeKind::Calls => !matches!(sym_kind, "namespace" | "module" | "package"),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "interface"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable" | "struct"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}
