/// Runtime globals always external for R.
///
/// R's C-level internal API names that appear in `.c`/`.cpp` files within R
/// packages (via R_RegisterCCallable, PROTECT, etc.). These are never defined
/// in R source and are not covered by primitives.rs.
pub(crate) const EXTERNALS: &[&str] = &[
    // C API — memory protection
    "PROTECT", "UNPROTECT", "UNPROTECT_PTR",
    // C API — type aliases / constructors
    "SEXP", "R_xlen_t", "SEXPREC",
    "ScalarInteger", "ScalarReal", "ScalarLogical", "ScalarString",
    "ScalarComplex", "ScalarRaw",
    "allocVector", "allocMatrix", "allocArray",
    "mkChar", "mkString",
    // C API — accessors
    "INTEGER", "REAL", "LOGICAL", "RAW", "COMPLEX", "STRING_ELT",
    "SET_STRING_ELT", "VECTOR_ELT", "SET_VECTOR_ELT",
    "LENGTH", "XLENGTH", "Rf_length",
    "TYPEOF", "NAMED",
    // C API — evaluation
    "eval", "Rf_eval", "R_tryEval",
    "CAR", "CDR", "CAAR", "CDAR", "CADR", "CDDR", "CADDR", "CDDDR",
    "CONS", "LCONS",
    // R_NilValue and friends
    "R_NilValue", "R_UnboundValue", "R_GlobalEnv", "R_BaseEnv",
    "R_EmptyEnv", "R_NaString", "R_BlankString",
    // C error / warning
    "Rf_error", "Rf_warning",
];

