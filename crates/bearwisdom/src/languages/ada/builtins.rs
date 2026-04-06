// =============================================================================
// ada/builtins.rs — Ada builtin and helper predicates
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

/// Ada predefined names from Standard and common runtime packages.
pub(super) fn is_ada_builtin(name: &str) -> bool {
    matches!(
        name,
        "Put"
            | "Put_Line"
            | "Get"
            | "Get_Line"
            | "New_Line"
            | "Integer"
            | "Float"
            | "Character"
            | "String"
            | "Boolean"
            | "Natural"
            | "Positive"
            | "Duration"
            | "True"
            | "False"
            | "Ada"
            | "System"
            | "Interfaces"
            | "GNAT"
            | "Standard"
            | "Text_IO"
            | "Integer_IO"
            | "Float_IO"
            | "Unchecked_Deallocation"
            | "Unchecked_Conversion"
    )
}
