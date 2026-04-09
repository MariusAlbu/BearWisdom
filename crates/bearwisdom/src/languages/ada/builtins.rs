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
        // I/O subprograms (Text_IO)
        "Put"
            | "Put_Line"
            | "Get"
            | "Get_Line"
            | "New_Line"
            | "Flush"
            // Scalar types from Standard
            | "Integer"
            | "Long_Integer"
            | "Short_Integer"
            | "Float"
            | "Long_Float"
            | "Short_Float"
            | "Character"
            | "Wide_Character"
            | "Wide_Wide_Character"
            | "String"
            | "Wide_String"
            | "Boolean"
            | "Natural"
            | "Positive"
            | "Duration"
            | "True"
            | "False"
            // Type attributes (commonly referenced as names)
            | "Integer'Image"
            | "Integer'Value"
            | "Float'Image"
            | "Float'Value"
            | "Boolean'Image"
            | "Boolean'Value"
            // Top-level library unit names
            | "Ada"
            | "System"
            | "Interfaces"
            | "GNAT"
            | "Standard"
            // Common child package short names (after `use`)
            | "Text_IO"
            | "Integer_IO"
            | "Float_IO"
            | "Unbounded"
            | "Unbounded_String"
            | "To_String"
            | "To_Unbounded_String"
            // Exceptions
            | "Constraint_Error"
            | "Program_Error"
            | "Storage_Error"
            | "Tasking_Error"
            | "Numeric_Error"
            // Unchecked operations
            | "Unchecked_Deallocation"
            | "Unchecked_Conversion"
    )
}
