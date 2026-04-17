// =============================================================================
// cobol/keywords.rs — COBOL primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for COBOL.
pub(crate) const KEYWORDS: &[&str] = &[
    // I/O verbs
    "DISPLAY", "ACCEPT",
    // arithmetic verbs
    "MOVE", "ADD", "SUBTRACT", "MULTIPLY", "DIVIDE", "COMPUTE",
    // flow
    "IF", "ELSE", "END-IF",
    "EVALUATE", "WHEN", "END-EVALUATE",
    "PERFORM", "END-PERFORM",
    "GO", "STOP", "EXIT", "GOBACK",
    // string / inspection verbs
    "INITIALIZE", "INSPECT", "STRING", "UNSTRING",
    // file verbs
    "OPEN", "CLOSE", "READ", "WRITE", "REWRITE", "DELETE",
    "START", "SEARCH", "SORT", "MERGE", "RELEASE", "RETURN",
    // misc verbs
    "SET", "CALL", "CANCEL", "INVOKE", "CONTINUE", "NEXT",
    "NOT", "ALTER", "GENERATE", "INITIATE", "TERMINATE",
    // picture / usage clauses
    "PIC", "PICTURE",
    "COMP", "COMP-1", "COMP-2", "COMP-3", "COMP-5",
    "BINARY", "PACKED-DECIMAL", "USAGE", "VALUE",
    "OCCURS", "REDEFINES", "FILLER",
    // data divisions
    "WORKING-STORAGE", "LOCAL-STORAGE", "LINKAGE", "FILE",
    // copy / replacing
    "COPY", "REPLACING",
    // intrinsic functions
    "FUNCTION", "LENGTH", "TRIM", "UPPER-CASE", "LOWER-CASE",
    "REVERSE", "NUMVAL", "NUMVAL-C", "ORD", "ORD-MIN", "ORD-MAX",
    "MAX", "MIN", "MEDIAN", "MEAN", "SUM",
    "INTEGER", "INTEGER-OF-DATE", "DATE-OF-INTEGER",
    "CURRENT-DATE", "WHEN-COMPILED",
    "RANDOM", "MOD", "REM", "FACTORIAL",
    "LOG", "LOG10", "SQRT",
    "SIN", "COS", "TAN", "ASIN", "ACOS", "ATAN",
    "ABS", "SIGN", "ANNUITY", "PRESENT-VALUE",
];
