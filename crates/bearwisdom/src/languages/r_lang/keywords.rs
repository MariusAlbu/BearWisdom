// =============================================================================
// r_lang/keywords.rs — R language keywords + interpreter primitives
//
// Names that are ALWAYS in scope and are implemented inside the R
// interpreter (C source, not walkable as R). Base/stats/utils functions
// that DO live as .R source under <R-src>/src/library/<pkg>/R/ (paste,
// sum, mean, sapply, plot, ...) are handled by the r_stdlib walker
// when BEARWISDOM_R_SRC points at an R source-distribution checkout.
// CRAN packages (testthat, dplyr, ggplot2, ...) are handled by the cran
// walker when DESCRIPTION declares them.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // Language constants
    "NULL", "NA", "NA_integer_", "NA_real_", "NA_complex_", "NA_character_",
    "TRUE", "FALSE", "T", "F",
    "Inf", "-Inf", "NaN", "pi",
    "LETTERS", "letters", "month.abb", "month.name",
    ".Machine", ".Platform",
    // Control-flow / declaration keywords
    "function", "return", "invisible",
    "if", "else", "for", "while", "repeat",
    "break", "next",
    "switch", "ifelse",
    "missing", "on.exit",
    // C-implemented primitive constructors (no .R source)
    "c", "list", "vector", "matrix", "array", "factor",
    "numeric", "integer", "character", "logical", "complex", "raw", "double",
    // C-implemented introspection / metaprogramming primitives
    "identical", "all.equal",
    "attr", "attributes", "structure",
    "class", "oldClass", "typeof", "mode", "storage.mode",
    "length", "lengths",
    "names", "rownames", "colnames", "dim", "dimnames",
    "environment", "new.env", "globalenv", "emptyenv", "baseenv",
    "parent.frame", "parent.env", "sys.call", "sys.frame", "sys.function",
    "match.call", "match.arg",
    "eval", "evalq", "parse", "deparse", "substitute", "quote", "bquote",
    "do.call", "Recall",
    "exists", "get", "mget", "assign", "rm", "remove",
    // C-implemented type predicates (always-in-scope, interpreter ops)
    "is.null", "is.na", "is.nan", "is.finite", "is.infinite",
    "is.numeric", "is.integer", "is.character", "is.logical", "is.complex",
    "is.list", "is.vector", "is.matrix", "is.array", "is.data.frame",
    "is.factor", "is.function", "is.environment", "is.symbol", "is.name",
    "is.atomic", "is.element", "is.expression", "is.language", "is.call",
    "is.pairlist", "is.primitive", "is.recursive", "is.object", "is.raw",
    "is.double", "is.single", "is.ordered", "is.qr", "is.unsorted",
    // C-implemented coercions
    "as.numeric", "as.integer", "as.character", "as.logical", "as.complex",
    "as.list", "as.vector", "as.matrix", "as.array", "as.data.frame",
    "as.factor", "as.function", "as.environment", "as.symbol", "as.name",
    "as.call", "as.expression", "as.raw", "as.double",
    // S3/S4/R5/R6 OO primitives
    "inherits", "UseMethod", "NextMethod", "standardGeneric",
    "setGeneric", "setMethod", "setClass", "new", "initialize",
    "validity", "is", "as", "slot", "slotNames", "hasSlot",
    "self", "private", "super", "active", "clone",
];
