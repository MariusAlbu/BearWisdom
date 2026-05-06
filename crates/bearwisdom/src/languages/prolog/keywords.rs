// =============================================================================
// prolog/keywords.rs — SWI-Prolog built-in predicates
//
// Names that the SWI-Prolog interpreter treats as always-in-scope built-ins.
// Library predicates that DO live as `.pl` source under
// `<swipl-root>/library/` are walked by prolog_runtime — these here are
// the interpreter-level predicates that have no walkable source.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // Control
    "true", "false", "fail", "halt",
    // I/O
    "write", "writeln", "writef", "read", "nl", "tab",
    // Type checks
    "atom", "number", "integer", "float", "compound",
    "is_list", "var", "nonvar",
    // Atom / string manipulation
    "atom_chars", "atom_length", "atom_concat", "atom_string",
    "number_chars", "number_codes", "char_code", "sub_atom",
    "atom_to_number", "term_to_atom", "term_string",
    "string_concat", "string_codes", "string_chars", "split_string",
    "string_to_atom", "atom_to_term",
    // Term inspection
    "functor", "arg", "copy_term", "ground", "callable",
    "number_vars", "numbervars",
    // Database manipulation
    "assert", "retract", "asserta", "assertz", "retractall", "abolish",
    // Aggregation / search
    "findall", "bagof", "setof", "aggregate_all", "forall", "between",
    // Arithmetic
    "succ", "plus", "is", "mod", "rem", "abs", "sign",
    "min", "max", "truncate", "round", "ceiling", "floor",
    "sqrt", "sin", "cos", "tan", "exp", "log", "random",
    // List predicates
    "length", "append", "member", "memberchk", "nth0", "nth1", "last",
    "msort", "sort", "predsort", "permutation", "flatten",
    "sumlist", "max_list", "min_list",
    "subtract", "intersection", "union",
    "select", "selectchk",
    "maplist", "include", "exclude", "foldl",
    // I/O terms
    "read_term", "write_term", "format",
];
