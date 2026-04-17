// =============================================================================
// r_lang/keywords.rs — R primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for R.
pub(crate) const KEYWORDS: &[&str] = &[
    // constructors
    "c", "list", "vector", "matrix", "array",
    "data.frame", "factor",
    "numeric", "integer", "character", "logical", "complex", "raw",
    // special values
    "NULL", "NA", "NA_integer_", "NA_real_", "NA_complex_", "NA_character_",
    "TRUE", "FALSE", "T", "F",
    "Inf", "-Inf", "NaN", "pi",
    "LETTERS", "letters", "month.abb", "month.name",
    // dimension / names
    "length", "nrow", "ncol", "dim", "nchar", "names",
    "colnames", "rownames", "class", "typeof", "mode",
    // type predicates
    "is.null", "is.na", "is.nan", "is.finite", "is.infinite",
    "is.numeric", "is.integer", "is.character", "is.logical",
    "is.list", "is.vector", "is.matrix", "is.data.frame",
    "is.factor", "is.function", "is.environment",
    // type coercions
    "as.numeric", "as.integer", "as.character", "as.logical",
    "as.list", "as.vector", "as.matrix", "as.data.frame", "as.factor",
    "as.Date", "as.POSIXct", "as.POSIXlt",
    // output / messaging
    "cat", "print", "paste", "paste0", "sprintf", "format", "formatC",
    "message", "warning", "stop",
    // condition system
    "tryCatch", "try", "withCallingHandlers",
    "simpleError", "simpleWarning", "simpleMessage",
    "conditionMessage", "conditionCall",
    // string operations
    "substr", "substring", "grep", "grepl", "sub", "gsub",
    "regexpr", "gregexpr", "regmatches", "strsplit",
    "toupper", "tolower", "trimws", "startsWith", "endsWith", "chartr",
    // file system
    "file.path", "file.exists", "file.remove", "file.rename",
    "file.copy", "file.create", "dir.create", "dir.exists",
    "basename", "dirname", "normalizePath", "path.expand",
    "getwd", "setwd", "tempdir", "tempfile",
    "Sys.getenv", "Sys.setenv", "Sys.time", "Sys.sleep",
    "system", "system2", "proc.time",
    // sequences
    "seq", "seq_len", "seq_along", "rep", "rep_len", "rev",
    "sort", "order", "rank",
    "which", "which.min", "which.max",
    "match", "pmatch", "charmatch",
    "duplicated", "unique", "table", "tabulate",
    // math / stats
    "sum", "prod", "cumsum", "cumprod", "cummax", "cummin", "diff",
    "range", "min", "max", "mean", "median", "var", "sd",
    "cor", "cov", "quantile",
    "abs", "sqrt", "exp", "log", "log2", "log10",
    "round", "floor", "ceiling", "trunc", "signif", "sign",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "choose", "factorial", "gamma", "lgamma", "beta", "lbeta", "combn",
    // random
    "sample", "set.seed",
    "runif", "rnorm", "rbinom", "rpois",
    "dnorm", "pnorm", "qnorm", "dbinom", "pbinom", "qbinom",
    // apply family
    "sapply", "lapply", "vapply", "tapply", "mapply", "apply",
    "Map", "Reduce", "Filter", "Find", "Position", "Negate", "do.call",
    // function introspection
    "on.exit", "sys.call", "match.arg", "missing", "hasArg",
    "nargs", "args", "formals", "body", "environment",
    // environment
    "parent.env", "new.env", "exists", "get", "mget", "assign",
    "rm", "ls", "objects", "search", "attach", "detach",
    // packages
    "require", "library", "installed.packages", "available.packages",
    "install.packages", "update.packages",
    "loadNamespace", "requireNamespace", "getNamespace", "isNamespace",
    "getOption", "options",
    // matrix operations
    "identical", "all.equal", "setdiff", "intersect", "union",
    "outer", "crossprod", "tcrossprod", "solve", "t", "det",
    "qr", "svd", "eigen", "chol", "backsolve", "forwardsolve",
    "diag", "upper.tri", "lower.tri",
    // factors / data
    "nlevels", "levels", "droplevels", "cut", "findInterval",
    // modeling / fitting
    "approx", "approxfun", "spline", "splinefun",
    "predict", "fitted", "residuals", "coef", "summary", "anova",
    "lm", "glm", "nls", "optim", "optimize", "nlm", "uniroot", "integrate",
    // graphics (base R)
    "plot", "lines", "points", "text", "title", "legend", "axis",
    "abline", "par", "layout",
    "pdf", "png", "svg", "dev.off",
    "hist", "barplot", "boxplot", "pie", "contour", "image", "persp",
    "curve", "qqnorm", "qqline", "qqplot", "pairs", "coplot",
    "stripchart", "dotchart", "sunflowerplot", "assocplot",
    "mosaicplot", "heatmap",
    // I/O
    "readline", "readLines", "writeLines", "scan",
    "read.table", "read.csv", "read.csv2", "read.delim", "read.delim2",
    "write.table", "write.csv", "write.csv2",
    "readRDS", "saveRDS", "load", "save", "source", "sink",
    "connection", "url", "file", "gzfile", "bzfile", "xzfile",
    "pipe", "textConnection", "rawConnection", "socketConnection",
    "open", "close", "readBin", "writeBin", "serialize", "unserialize",
    "dput", "dump",
    // debugging
    "str", "head", "tail", "View", "edit", "fix",
    "debug", "undebug", "debugonce", "browser", "traceback",
    "trace", "untrace", "recover",
    // misc
    "return", "invisible", "switch", "ifelse",
    // S3/S4/R5/R6 OO
    "inherits", "UseMethod", "NextMethod", "standardGeneric",
    "setGeneric", "setMethod", "setClass", "new", "initialize", "show",
    "validity", "is", "as", "slot", "slotNames", "hasSlot",
    "existsMethod", "isVirtualClass",
    "R6Class", "R6", "self", "private", "super", "active", "clone",
    // testthat
    "test_that", "expect_equal", "expect_identical",
    "expect_true", "expect_false", "expect_null",
    "expect_error", "expect_warning", "expect_message",
    "expect_condition", "expect_output", "expect_silent",
    "expect_invisible", "expect_visible",
    "expect_type", "expect_s3_class", "expect_s4_class",
    "expect_length", "expect_match", "expect_named",
    "expect_setequal", "expect_mapequal",
    "expect_gt", "expect_gte", "expect_lt", "expect_lte",
    "expect_snapshot", "expect_snapshot_output",
    "expect_snapshot_error", "expect_snapshot_value",
    "expect_no_error", "expect_no_warning",
    "expect_no_message", "expect_no_condition",
    "skip", "skip_if", "skip_if_not",
    "skip_on_cran", "skip_on_ci", "skip_on_os",
    "describe", "it", "context", "setup", "teardown",
    "test_path", "test_check", "test_file", "test_dir",
    "local_edition", "with_mocked_bindings",
];
