// =============================================================================
// prolog/builtins.rs — Prolog builtin and helper predicates
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

/// Prolog ISO and SWI-Prolog standard builtins always in scope.
pub(super) fn is_prolog_builtin(name: &str) -> bool {
    matches!(
        name,
        // Control
        "true"
            | "false"
            | "fail"
            | "halt"
            // I/O
            | "write"
            | "writeln"
            | "writef"
            | "read"
            | "nl"
            | "tab"
            // Type checks
            | "atom"
            | "number"
            | "integer"
            | "float"
            | "compound"
            | "is_list"
            | "var"
            | "nonvar"
            // Atom / string manipulation
            | "atom_chars"
            | "atom_length"
            | "atom_concat"
            | "atom_string"
            | "number_chars"
            | "number_codes"
            | "char_code"
            | "sub_atom"
            | "atom_to_number"
            | "term_to_atom"
            | "term_string"
            | "string_concat"
            | "string_codes"
            | "string_chars"
            | "split_string"
            | "string_to_atom"
            | "atom_to_term"
            // Term inspection
            | "functor"
            | "arg"
            | "copy_term"
            | "ground"
            | "callable"
            | "number_vars"
            | "numbervars"
            // Database manipulation
            | "assert"
            | "retract"
            | "asserta"
            | "assertz"
            | "retractall"
            | "abolish"
            // Aggregation / search
            | "findall"
            | "bagof"
            | "setof"
            | "aggregate_all"
            | "forall"
            | "between"
            // Arithmetic
            | "succ"
            | "plus"
            | "is"
            | "mod"
            | "rem"
            | "abs"
            | "sign"
            | "min"
            | "max"
            | "truncate"
            | "round"
            | "ceiling"
            | "floor"
            | "sqrt"
            | "sin"
            | "cos"
            | "tan"
            | "exp"
            | "log"
            | "random"
            // List predicates
            | "length"
            | "append"
            | "member"
            | "memberchk"
            | "nth0"
            | "nth1"
            | "last"
            | "msort"
            | "sort"
            | "predsort"
            | "permutation"
            | "flatten"
            | "sumlist"
            | "max_list"
            | "min_list"
            | "subtract"
            | "intersection"
            | "union"
            | "select"
            | "selectchk"
            | "maplist"
            | "include"
            | "exclude"
            | "foldl"
            // I/O terms
            | "read_term"
            | "write_term"
            | "format"
    )
}
