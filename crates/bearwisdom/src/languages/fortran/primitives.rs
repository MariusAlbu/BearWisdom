// =============================================================================
// fortran/primitives.rs — Fortran primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Fortran.
pub(crate) const PRIMITIVES: &[&str] = &[
    // intrinsic functions
    "abs", "achar", "acos", "adjustl", "adjustr", "aimag", "aint",
    "all", "allocated", "anint", "any", "asin", "associated",
    "atan", "atan2", "bit_size", "btest", "ceiling", "char", "cmplx",
    "conjg", "cos", "cosh", "count", "cshift", "dble", "digits", "dim",
    "dot_product", "dprod", "eoshift", "epsilon", "exp", "exponent",
    "floor", "fraction", "huge", "iachar", "iand", "ibclr", "ibits",
    "ibset", "ichar", "ieor", "index", "int", "ior", "ishft", "ishftc",
    "kind", "lbound", "len", "len_trim", "lge", "lgt", "lle", "llt",
    "log", "log10", "matmul", "max", "maxloc", "maxval", "merge",
    "min", "minloc", "minval", "mod", "modulo", "mvbits", "nearest",
    "nint", "not", "pack", "precision", "present", "product",
    "radix", "range", "real", "repeat", "reshape", "rrspacing",
    "scale", "scan", "selected_int_kind", "selected_real_kind",
    "set_exponent", "shape", "sign", "sin", "sinh", "size",
    "spacing", "spread", "sqrt", "sum", "tan", "tanh", "tiny",
    "transfer", "transpose", "trim", "ubound", "unpack", "verify",
    "ieee_is_nan", "norm2", "findloc", "is_contiguous", "new_line",
    "null", "move_alloc",
    "command_argument_count", "get_command_argument",
    "execute_command_line",
    // type specifiers
    "integer", "real", "double", "complex", "character", "logical",
    "class", "type", "dimension", "allocatable", "pointer", "target",
    "intent", "optional", "save", "parameter", "implicit", "none",
    "private", "public", "protected",
    "abstract", "deferred", "extends", "generic", "final",
    "non_overridable", "sequence", "bind", "value", "volatile",
    "asynchronous", "contiguous", "codimension",
    // I/O statements
    "print", "write", "read", "open", "close", "rewind",
    "backspace", "endfile", "inquire", "format",
    // control flow
    "stop", "error", "return", "cycle", "exit",
    "call", "subroutine", "function", "program", "module",
    "use", "only", "contains", "interface", "end",
    "do", "if", "then", "else", "elseif",
    "select", "case", "where", "forall", "associate",
    "block", "critical", "sync",
    "allocate", "deallocate",
];
