// =============================================================================
// nim/predicates.rs — Nim builtin and helper predicates
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

/// Nim builtin functions and types always in scope.
pub(super) fn is_nim_builtin(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | "debugEcho"
            | "len"
            | "high"
            | "low"
            | "inc"
            | "dec"
            | "add"
            | "del"
            | "contains"
            | "assert"
            | "doAssert"
            | "newSeq"
            | "newString"
            | "repr"
            | "quit"
            | "ord"
            | "chr"
            | "parseInt"
            | "parseFloat"
            | "split"
            | "join"
            | "strip"
            | "find"
            | "replace"
            | "startsWith"
            | "endsWith"
            | "toUpper"
            | "toLower"
            | "isNil"
    )
}
