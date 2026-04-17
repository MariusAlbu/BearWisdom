// =============================================================================
// cobol/predicates.rs — COBOL builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(sym_kind, "class" | "interface" | "enum" | "type_alias" | "function" | "variable"),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// COBOL standard verbs and intrinsic functions always in scope.
pub(super) fn is_cobol_builtin(name: &str) -> bool {
    matches!(
        name,
        // Standard verbs
        "DISPLAY"
            | "ACCEPT"
            | "MOVE"
            | "ADD"
            | "SUBTRACT"
            | "MULTIPLY"
            | "DIVIDE"
            | "COMPUTE"
            | "IF"
            | "EVALUATE"
            | "PERFORM"
            | "GO"
            | "STOP"
            | "EXIT"
            | "CALL"
            | "CANCEL"
            | "GOBACK"
            | "INITIALIZE"
            | "INSPECT"
            | "STRING"
            | "UNSTRING"
            | "SEARCH"
            | "SORT"
            | "MERGE"
            | "OPEN"
            | "CLOSE"
            | "READ"
            | "WRITE"
            | "REWRITE"
            | "DELETE"
            | "START"
            | "RELEASE"
            | "RETURN"
            | "SET"
            | "ALLOCATE"
            | "FREE"
            | "RAISE"
            | "CONTINUE"
            | "NEXT"
            // Intrinsic functions (used via FUNCTION keyword)
            | "FUNCTION"
            | "LENGTH"
            | "REVERSE"
            | "UPPER-CASE"
            | "LOWER-CASE"
            | "TRIM"
            | "NUMVAL"
            | "NUMVAL-C"
            | "INTEGER"
            | "INTEGER-OF-DATE"
            | "DATE-OF-INTEGER"
            | "CURRENT-DATE"
            | "WHEN-COMPILED"
            | "MAX"
            | "MIN"
            | "ORD"
            | "ORD-MAX"
            | "ORD-MIN"
            | "SUM"
            | "MEAN"
            | "MEDIAN"
            | "MIDRANGE"
            | "PRESENT-VALUE"
            | "RANDOM"
            | "REM"
            | "MOD"
            | "FACTORIAL"
            | "ANNUITY"
            | "SQRT"
            | "LOG"
            | "LOG10"
            | "EXP"
            | "EXP10"
            | "SIN"
            | "COS"
            | "TAN"
            | "ASIN"
            | "ACOS"
            | "ATAN"
    )
}
