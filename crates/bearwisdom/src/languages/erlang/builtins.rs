// =============================================================================
// erlang/builtins.rs — Erlang builtin and helper predicates
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

/// Erlang built-in functions and standard library modules always in scope.
pub(super) fn is_erlang_builtin(name: &str) -> bool {
    matches!(
        name,
        // OTP modules
        "erlang"
            | "io"
            | "lists"
            | "maps"
            | "sets"
            | "dict"
            | "ets"
            | "gen_server"
            | "gen_statem"
            | "supervisor"
            | "application"
            | "io_lib"
            | "string"
            | "binary"
            | "file"
            | "timer"
            | "crypto"
            | "ssl"
            | "gen_tcp"
            | "gen_udp"
            | "inet"
            | "proplists"
            | "re"
            | "calendar"
            | "filename"
            | "code"
            | "error_logger"
            | "logger"
            // BIFs — process / concurrency
            | "self"
            | "spawn"
            | "send"
            | "receive"
            | "exit"
            | "throw"
            | "catch"
            | "try"
            | "begin"
            | "end"
            // BIFs — type tests
            | "is_atom"
            | "is_binary"
            | "is_boolean"
            | "is_float"
            | "is_function"
            | "is_integer"
            | "is_list"
            | "is_map"
            | "is_number"
            | "is_pid"
            | "is_port"
            | "is_reference"
            | "is_tuple"
            // BIFs — data manipulation
            | "hd"
            | "tl"
            | "length"
            | "element"
            | "setelement"
            | "tuple_size"
            | "map_size"
            | "abs"
            | "max"
            | "min"
            | "round"
            | "trunc"
            | "float"
            | "list_to_atom"
            | "atom_to_list"
            | "list_to_binary"
            | "binary_to_list"
            | "integer_to_list"
            | "list_to_integer"
            | "term_to_binary"
            | "binary_to_term"
            | "iolist_to_binary"
            // BIFs — process dictionary
            | "put"
            | "get"
            | "erase"
    )
}
