// =============================================================================
// ocaml/builtins.rs — OCaml builtin and helper predicates
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

/// OCaml Stdlib functions and modules always in scope.
pub(super) fn is_ocaml_builtin(name: &str) -> bool {
    matches!(
        name,
        // I/O
        "print_string"
            | "print_endline"
            | "print_int"
            | "print_float"
            | "print_char"
            | "print_newline"
            | "prerr_string"
            | "prerr_endline"
            | "read_line"
            | "read_int"
            | "read_float"
            | "input_line"
            | "output_string"
            // Conversion
            | "string_of_int"
            | "int_of_string"
            | "string_of_float"
            | "float_of_string"
            | "string_of_bool"
            | "bool_of_string"
            | "char_of_int"
            | "int_of_char"
            // Control
            | "ignore"
            | "failwith"
            | "invalid_arg"
            | "raise"
            | "assert"
            | "fst"
            | "snd"
            // Numeric
            | "min"
            | "max"
            | "abs"
            | "succ"
            | "pred"
            | "mod_float"
            | "sqrt"
            | "exp"
            | "log"
            | "log10"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
            | "ceil"
            | "floor"
            | "truncate"
            | "float_of_int"
            | "int_of_float"
            // References
            | "ref"
            | "incr"
            | "decr"
            | "not"
            | "compare"
            // Stdlib modules
            | "List"
            | "Array"
            | "String"
            | "Bytes"
            | "Buffer"
            | "Hashtbl"
            | "Map"
            | "Set"
            | "Stack"
            | "Queue"
            | "Stream"
            | "Scanf"
            | "Printf"
            | "Format"
            | "Sys"
            | "Unix"
            | "Filename"
            | "Arg"
            | "Printexc"
            | "Lazy"
            | "Fun"
            | "Seq"
            | "Option"
            | "Result"
            | "Either"
            // Constructors
            | "Some"
            | "None"
            | "Ok"
            | "Error"
            // Literals
            | "true"
            | "false"
            | "unit"
    )
}
