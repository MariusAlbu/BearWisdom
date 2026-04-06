// =============================================================================
// r_lang/builtins.rs — R builtin and helper predicates
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

/// R base functions, constants, and primitives that are never in the project index.
pub(super) fn is_r_builtin(name: &str) -> bool {
    matches!(
        name,
        "c"
            | "list"
            | "data.frame"
            | "matrix"
            | "array"
            | "vector"
            | "print"
            | "cat"
            | "paste"
            | "paste0"
            | "sprintf"
            | "length"
            | "nrow"
            | "ncol"
            | "dim"
            | "names"
            | "which"
            | "match"
            | "seq"
            | "rep"
            | "sum"
            | "mean"
            | "median"
            | "sd"
            | "var"
            | "min"
            | "max"
            | "range"
            | "abs"
            | "sqrt"
            | "log"
            | "exp"
            | "ceiling"
            | "floor"
            | "round"
            | "is.na"
            | "is.null"
            | "is.numeric"
            | "as.character"
            | "as.numeric"
            | "as.integer"
            | "as.logical"
            | "ifelse"
            | "switch"
            | "apply"
            | "sapply"
            | "lapply"
            | "tapply"
            | "vapply"
            | "do.call"
            | "tryCatch"
            | "stop"
            | "warning"
            | "message"
            | "function"
            | "return"
            | "invisible"
            | "TRUE"
            | "FALSE"
            | "NULL"
            | "NA"
            | "Inf"
            | "NaN"
            | "pi"
    )
}
