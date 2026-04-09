/// SWI-Prolog standard library predicates, SICStus extensions, and constraint
/// library predicates — always external (never defined inside a project).
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // library(lists) — list predicates
    // -------------------------------------------------------------------------
    "append",
    "member",
    "memberchk",
    "msort",
    "permutation",
    "nth0",
    "nth1",
    "last",
    "flatten",
    "sumlist",
    "sum_list",
    "max_list",
    "min_list",
    "numlist",
    "list_to_set",
    "delete",
    "subtract",
    "intersection",
    "union",
    "select",
    "selectchk",
    "nextto",
    "permutation",
    "prefix",
    "suffix",
    // -------------------------------------------------------------------------
    // library(apply) — higher-order predicates
    // -------------------------------------------------------------------------
    "maplist",
    "include",
    "exclude",
    "foldl",
    "aggregate_all",
    "foreach",
    // -------------------------------------------------------------------------
    // library(aggregate)
    // -------------------------------------------------------------------------
    "aggregate",
    "aggregate_all",
    "foreach",
    // -------------------------------------------------------------------------
    // library(http/http_open) — HTTP client
    // -------------------------------------------------------------------------
    "http_open",
    "http_get",
    "http_post",
    "http_put",
    "http_delete",
    "http_read_data",
    // -------------------------------------------------------------------------
    // library(clpfd) — Constraint Logic Programming over Finite Domains
    // -------------------------------------------------------------------------
    "ins",
    "in",
    "all_distinct",
    "all_different",
    "sum",
    "scalar_product",
    "tuples_in",
    "label",
    "labeling",
    "indomain",
    "fd_dom",
    "fd_size",
    "fd_inf",
    "fd_sup",
    // -------------------------------------------------------------------------
    // library(clpb) — Boolean CLP
    // -------------------------------------------------------------------------
    "sat",
    "taut",
    "labeling",
    "sat_count",
    // -------------------------------------------------------------------------
    // library(pairs) — key-value pairs
    // -------------------------------------------------------------------------
    "pairs_keys",
    "pairs_values",
    "pairs_keys_values",
    "list_to_assoc",
    "assoc_to_list",
    "get_assoc",
    "put_assoc",
    // -------------------------------------------------------------------------
    // library(readutil)
    // -------------------------------------------------------------------------
    "read_term_from_atom",
    "read_file_to_terms",
    "read_file_to_string",
    // -------------------------------------------------------------------------
    // library(string) / atom helpers
    // -------------------------------------------------------------------------
    "string_codes",
    "string_chars",
    "string_concat",
    "string_length",
    "string_lower",
    "string_upper",
    "string_to_atom",
    "number_string",
    "atom_string",
    // -------------------------------------------------------------------------
    // SICStus Prolog extensions
    // -------------------------------------------------------------------------
    "succ_or_zero",
    "between",
    "numlist",
    "msort",
    "predsort",
    "nb_getval",
    "nb_setval",
    "b_getval",
    "b_setval",
];
