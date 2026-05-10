// =============================================================================
// fortran/keywords.rs — Fortran grammar keywords and compiler intrinsics.
//
// Two categories belong here:
//
//  1. Grammar keywords — reserved words that structure Fortran programs
//     (do/end/if/module/use/...) and type specifiers (integer/real/...).
//     The engine suppresses unresolved-ref noise for all of these.
//
//  2. Compiler intrinsics — procedures and functions handled entirely by
//     the Fortran compiler with no source declaration in any module file.
//     They are implicitly available everywhere without a USE statement.
//     Standard references: Fortran 2018 §16 (intrinsic procedures) and
//     the GNU Fortran intrinsic reference.
//
// What does NOT belong here: procedures from the Fortran standard library
// (stdlib), LAPACK, BLAS, or any package that ships actual .f90 source.
// Those are resolved through the symbol index once their source is indexed.
// =============================================================================

/// Grammar keywords and compiler intrinsic names for Fortran.
/// All entries are lowercase — Fortran is case-insensitive and refs are
/// matched with a case-folded comparison in the resolver.
pub(crate) const KEYWORDS: &[&str] = &[
    // ── Compiler intrinsics — math ───────────────────────────────────────────
    // Elemental numeric functions with no declaration in any source file.
    "abs", "aimag", "aint", "anint", "ceiling", "conjg", "dim",
    "dprod", "floor", "fraction", "huge", "max", "min", "mod", "modulo",
    "nearest", "nint", "rrspacing", "scale", "sign", "sin", "sinh",
    "spacing", "sqrt", "tan", "tanh", "tiny", "trunc",
    "acos", "asin", "atan", "atan2", "cos", "cosh", "exp", "exponent",
    "log", "log10", "norm2",

    // ── Compiler intrinsics — type conversion ────────────────────────────────
    "cmplx", "dble", "float", "ifix", "idint", "int", "nint", "real",
    "transfer",

    // ── Compiler intrinsics — type inquiry ───────────────────────────────────
    // Return compile-time model parameters; no source declaration exists.
    "bit_size", "digits", "epsilon", "kind", "maxexponent", "minexponent",
    "precision", "radix", "range", "selected_int_kind", "selected_real_kind",
    "set_exponent",

    // ── Compiler intrinsics — string ─────────────────────────────────────────
    "achar", "adjustl", "adjustr", "char", "iachar", "ichar", "index",
    "len", "len_trim", "lge", "lgt", "lle", "llt", "new_line", "repeat",
    "scan", "trim", "verify",

    // ── Compiler intrinsics — array ───────────────────────────────────────────
    "all", "any", "count", "cshift", "dot_product", "eoshift", "findloc",
    "lbound", "matmul", "maxloc", "maxval", "merge", "minloc", "minval",
    "pack", "product", "reshape", "shape", "size", "spread", "sum",
    "transpose", "ubound", "unpack", "is_contiguous",

    // ── Compiler intrinsics — bit manipulation ────────────────────────────────
    "btest", "iand", "ibclr", "ibits", "ibset", "ieor", "ior", "ishft",
    "ishftc", "mvbits", "not",

    // ── Compiler intrinsics — pointer / allocation ────────────────────────────
    "allocated", "associated", "move_alloc", "null", "present",

    // ── Compiler intrinsics — I/O and environment ────────────────────────────
    // Called via CALL or as functions; the compiler handles them with no
    // module declaration.
    "command_argument_count", "cpu_time", "date_and_time",
    "execute_command_line", "get_command", "get_command_argument",
    "get_environment_variable", "random_number", "random_seed",
    "system_clock",

    // ── Compiler intrinsics — IEEE arithmetic (ieee_arithmetic module) ────────
    // These are intrinsic module procedures with no source declaration.
    // The compiler supplies the ieee_arithmetic module; these symbols are
    // callable without any external library.
    "ieee_value", "ieee_quiet_nan", "ieee_positive_inf", "ieee_negative_inf",
    "ieee_support_inf", "ieee_support_nan", "ieee_support_halting",
    "ieee_support_rounding", "ieee_support_sqrt", "ieee_is_nan",
    "ieee_is_finite", "ieee_is_negative", "ieee_is_normal",
    "ieee_class", "ieee_copy_sign", "ieee_logb", "ieee_next_after",
    "ieee_rem", "ieee_rint", "ieee_scalb", "ieee_unordered",
    "ieee_get_flag", "ieee_get_halting_mode", "ieee_get_rounding_mode",
    "ieee_set_flag", "ieee_set_halting_mode", "ieee_set_rounding_mode",

    // ── Type specifiers ───────────────────────────────────────────────────────
    "integer", "real", "double", "complex", "character", "logical",
    "class", "type", "dimension", "allocatable", "pointer", "target",
    "intent", "optional", "save", "parameter", "implicit", "none",
    "private", "public", "protected",
    "abstract", "deferred", "extends", "generic", "final",
    "non_overridable", "sequence", "bind", "value", "volatile",
    "asynchronous", "contiguous", "codimension",

    // ── I/O statements ────────────────────────────────────────────────────────
    "print", "write", "read", "open", "close", "rewind",
    "backspace", "endfile", "inquire", "format",

    // ── Control flow ─────────────────────────────────────────────────────────
    "stop", "error", "return", "cycle", "exit",
    "call", "subroutine", "function", "program", "module",
    "use", "only", "contains", "interface", "end",
    "do", "if", "then", "else", "elseif",
    "select", "case", "where", "forall", "associate",
    "block", "critical", "sync",
    "allocate", "deallocate",
];
