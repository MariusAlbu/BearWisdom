use std::collections::HashSet;

/// Runtime globals always external for OCaml.
///
/// With the import walk in `infer_external_common`, module-qualified names
/// (List.map, Map.find, Fmt.pr) and test framework names are now classified
/// via their `open` statements. This list retains only Stdlib/Pervasives
/// names that are always in scope without any `open`.
pub(crate) const EXTERNALS: &[&str] = &[
    // --- Option / Result constructors (always in scope, no import) ---
    "Some", "None", "Ok", "Error",

    // --- Stdlib / Pervasives (always in scope) ---
    "failwith", "invalid_arg", "raise", "raise_notrace",
    "not", "fst", "snd",
    "ref", "ignore", "incr", "decr",
    "succ", "pred", "abs",
    "min", "max", "compare",
    "print_endline", "print_string", "print_char",
    "print_int", "print_float", "print_newline",
    "prerr_endline", "prerr_string",
    "read_line", "read_int", "read_float",
    "exit", "at_exit",
    "string_of_int", "string_of_float", "string_of_bool",
    "int_of_string", "float_of_string", "bool_of_string",
    "int_of_char", "char_of_int",
    "truncate", "float_of_int", "int_of_float",
    "sqrt", "exp", "log", "log10",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "ceil", "floor",
    "mod_float", "ldexp", "frexp", "modf",
    "infinity", "nan", "epsilon_float",
    "min_int", "max_int", "min_float", "max_float",
    "format_of_string", "string_to_bytes", "bytes_to_string",
    "assert", "fun",
];

/// Dependency-gated framework globals for OCaml.
///
/// Import walk now handles test framework names via `open Alcotest` etc.
pub(crate) fn framework_globals(_deps: &HashSet<String>) -> Vec<&'static str> {
    Vec::new()
}
