// =============================================================================
// fortran/builtins.rs — Fortran builtin and helper predicates
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

/// Fortran intrinsic procedures and I/O statements always in scope.
pub(super) fn is_fortran_builtin(name: &str) -> bool {
    // Case-insensitive match: Fortran is case-insensitive, so we check the
    // lowercased form. Callers should pass already-lowercased names, or we
    // match both cases for the most common mixed-case variants.
    matches!(
        name,
        "write"
            | "read"
            | "print"
            | "open"
            | "close"
            | "allocate"
            | "deallocate"
            | "associated"
            | "present"
            | "size"
            | "shape"
            | "lbound"
            | "ubound"
            | "len"
            | "trim"
            | "adjustl"
            | "adjustr"
            | "index"
            | "scan"
            | "verify"
            | "repeat"
            | "transfer"
            | "real"
            | "int"
            | "dble"
            | "cmplx"
            | "abs"
            | "sqrt"
            | "sin"
            | "cos"
            | "tan"
            | "exp"
            | "log"
            | "log10"
            | "max"
            | "min"
            | "mod"
            | "sum"
            | "product"
            | "matmul"
            | "dot_product"
            | "transpose"
            | "reshape"
            | "pack"
            | "unpack"
            | "merge"
            | "spread"
            | "cshift"
            | "eoshift"
            | "maxval"
            | "minval"
            | "maxloc"
            | "minloc"
            | "any"
            | "all"
            | "count"
            // Uppercase variants (Fortran legacy style)
            | "WRITE"
            | "READ"
            | "PRINT"
            | "OPEN"
            | "CLOSE"
            | "ALLOCATE"
            | "DEALLOCATE"
            | "ASSOCIATED"
            | "PRESENT"
            | "SIZE"
            | "SHAPE"
            | "LBOUND"
            | "UBOUND"
            | "LEN"
            | "TRIM"
            | "ABS"
            | "SQRT"
            | "SIN"
            | "COS"
            | "TAN"
            | "EXP"
            | "LOG"
            | "MAX"
            | "MIN"
            | "MOD"
            | "SUM"
            | "PRODUCT"
            | "MATMUL"
            | "TRANSPOSE"
            | "RESHAPE"
            | "ANY"
            | "ALL"
            | "COUNT"
    )
}
