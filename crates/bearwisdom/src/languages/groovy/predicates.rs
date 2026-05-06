// =============================================================================
// groovy/predicates.rs — Groovy builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
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

/// Groovy control flow keywords that the grammar may parse as method_invocation.
/// Used by the extractor to filter parser-noise refs at extract time.
pub(super) fn is_groovy_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "else" | "while" | "for" | "switch" | "case" | "do"
            | "try" | "catch" | "finally" | "throw" | "return"
            | "break" | "continue" | "assert"
    )
}
