// =============================================================================
// ada/predicates.rs — Ada builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// Ada-specific looseness on `Calls`: the language conflates many forms
/// behind the parens-everywhere rule. `This.CCER (Channel)` looks like a
/// procedure call but is array indexing into a record field. `Convert
/// (Integer_32, X)` instantiates a generic with type arguments. The
/// extractor emits all these as Calls — which strictly should reject
/// matching against `variable`, `field`, `struct`, `enum`, `type_alias`.
/// We allow them so the resolver can still attribute the ref to the
/// actual symbol the user wrote, even if the EdgeKind classification
/// from tree-sitter is overly broad.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method"
                | "function"
                | "constructor"
                | "test"
                | "class"
                | "namespace"
                | "variable"
                | "field"
                | "struct"
                | "enum"
                | "type_alias"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function" | "namespace"),
        _ => true,
    }
}

