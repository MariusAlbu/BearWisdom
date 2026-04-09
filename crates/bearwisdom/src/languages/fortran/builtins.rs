// =============================================================================
// fortran/builtins.rs — Fortran builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Fortran intrinsic procedures and I/O statements always in scope.
///
/// Fortran is case-insensitive; callers either pass the name as-is and rely on
/// the resolve.rs wrapper `is_fortran_builtin_ci`, or lowercase first.
/// The match covers both lowercase and uppercase for the most common intrinsics.
pub(super) fn is_fortran_builtin(name: &str) -> bool {
    matches!(
        name,
        // ── I/O statements ────────────────────────────────────────────────────
        "write" | "read" | "print" | "open" | "close" | "flush"
            | "rewind" | "backspace" | "inquire"
            | "WRITE" | "READ" | "PRINT" | "OPEN" | "CLOSE" | "FLUSH"
            | "REWIND" | "BACKSPACE" | "INQUIRE"
        // ── Memory management ─────────────────────────────────────────────────
            | "allocate" | "deallocate" | "nullify" | "move_alloc"
            | "ALLOCATE" | "DEALLOCATE" | "NULLIFY" | "MOVE_ALLOC"
        // ── Inquiry / array shape ─────────────────────────────────────────────
            | "allocated" | "associated" | "present"
            | "size" | "shape" | "lbound" | "ubound" | "rank"
            | "len" | "len_trim"
            | "storage_size" | "c_sizeof"
            | "is_contiguous"
            | "ALLOCATED" | "ASSOCIATED" | "PRESENT"
            | "SIZE" | "SHAPE" | "LBOUND" | "UBOUND" | "RANK"
            | "LEN" | "LEN_TRIM" | "STORAGE_SIZE"
        // ── Math ─────────────────────────────────────────────────────────────
            | "abs" | "aint" | "anint" | "ceiling" | "floor" | "nint" | "round"
            | "mod" | "modulo" | "sign" | "dim" | "dprod"
            | "max" | "min"
            | "sqrt" | "exp" | "log" | "log10"
            | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
            | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh"
            | "hypot" | "norm2"
            | "erf" | "erfc" | "erfc_scaled" | "gamma" | "log_gamma"
            | "ABS" | "AINT" | "ANINT" | "CEILING" | "FLOOR" | "NINT" | "ROUND"
            | "MOD" | "MODULO" | "SIGN" | "DIM" | "DPROD"
            | "MAX" | "MIN"
            | "SQRT" | "EXP" | "LOG" | "LOG10"
            | "SIN" | "COS" | "TAN" | "ASIN" | "ACOS" | "ATAN" | "ATAN2"
            | "SINH" | "COSH" | "TANH" | "ASINH" | "ACOSH" | "ATANH"
            | "HYPOT" | "NORM2"
        // ── Complex ───────────────────────────────────────────────────────────
            | "aimag" | "conjg" | "real" | "cmplx"
            | "AIMAG" | "CONJG" | "REAL" | "CMPLX"
        // ── Type conversion / kind ────────────────────────────────────────────
            | "int" | "dble" | "ichar" | "char" | "achar" | "iachar"
            | "kind" | "selected_real_kind" | "selected_int_kind" | "selected_char_kind"
            | "range" | "precision" | "radix" | "digits"
            | "epsilon" | "huge" | "tiny" | "bit_size"
            | "minexponent" | "maxexponent"
            | "new_line"
            | "INT" | "DBLE" | "ICHAR" | "CHAR" | "ACHAR" | "IACHAR"
            | "KIND" | "SELECTED_REAL_KIND" | "SELECTED_INT_KIND"
            | "RANGE" | "PRECISION" | "RADIX" | "DIGITS"
            | "EPSILON" | "HUGE" | "TINY" | "BIT_SIZE"
        // ── Character / string ────────────────────────────────────────────────
            | "trim" | "adjustl" | "adjustr" | "index" | "scan" | "verify" | "repeat"
            | "TRIM" | "ADJUSTL" | "ADJUSTR" | "INDEX" | "SCAN" | "VERIFY" | "REPEAT"
        // ── Transfer / reshape ────────────────────────────────────────────────
            | "transfer" | "reshape"
            | "TRANSFER" | "RESHAPE"
        // ── Array reduction ───────────────────────────────────────────────────
            | "sum" | "product" | "count" | "any" | "all"
            | "maxval" | "minval" | "maxloc" | "minloc" | "findloc"
            | "dot_product" | "matmul" | "transpose"
            | "spread" | "pack" | "unpack"
            | "cshift" | "eoshift" | "merge" | "parity"
            | "SUM" | "PRODUCT" | "COUNT" | "ANY" | "ALL"
            | "MAXVAL" | "MINVAL" | "MAXLOC" | "MINLOC" | "FINDLOC"
            | "DOT_PRODUCT" | "MATMUL" | "TRANSPOSE"
            | "SPREAD" | "PACK" | "UNPACK"
            | "CSHIFT" | "EOSHIFT" | "MERGE" | "PARITY"
        // ── Bit intrinsics ────────────────────────────────────────────────────
            | "iand" | "ior" | "ieor" | "not" | "btest" | "ibset" | "ibclr"
            | "ishft" | "ishftc" | "ibits" | "mvbits"
            | "leadz" | "trailz" | "popcnt" | "poppar"
            | "maskl" | "maskr"
            | "shifta" | "shiftl" | "shiftr" | "dshiftl" | "dshiftr"
            | "merge_bits"
            | "bge" | "bgt" | "ble" | "blt"
            | "IAND" | "IOR" | "IEOR" | "NOT" | "BTEST" | "IBSET" | "IBCLR"
            | "ISHFT" | "ISHFTC" | "IBITS" | "MVBITS"
            | "LEADZ" | "TRAILZ" | "POPCNT" | "POPPAR"
            | "MASKL" | "MASKR"
            | "SHIFTA" | "SHIFTL" | "SHIFTR" | "DSHIFTL" | "DSHIFTR"
            | "MERGE_BITS"
            | "BGE" | "BGT" | "BLE" | "BLT"
        // ── System / runtime ──────────────────────────────────────────────────
            | "stop" | "error_stop" | "exit"
            | "system_clock" | "date_and_time" | "cpu_time"
            | "random_number" | "random_seed"
            | "get_command" | "get_command_argument" | "get_environment_variable"
            | "command_argument_count" | "execute_command_line"
            | "STOP" | "ERROR_STOP" | "EXIT"
            | "SYSTEM_CLOCK" | "DATE_AND_TIME" | "CPU_TIME"
            | "RANDOM_NUMBER" | "RANDOM_SEED"
            | "GET_COMMAND_ARGUMENT" | "GET_ENVIRONMENT_VARIABLE"
            | "COMMAND_ARGUMENT_COUNT" | "EXECUTE_COMMAND_LINE"
        // ── C interoperability ────────────────────────────────────────────────
            | "c_loc" | "c_funloc" | "c_f_pointer" | "c_f_procpointer"
            | "c_associated"
            | "C_LOC" | "C_FUNLOC" | "C_F_POINTER" | "C_ASSOCIATED"
        // ── Coarray ───────────────────────────────────────────────────────────
            | "co_sum" | "co_broadcast" | "co_max" | "co_min" | "co_reduce"
            | "this_image" | "num_images" | "image_index"
            | "CO_SUM" | "CO_BROADCAST" | "CO_MAX" | "CO_MIN" | "CO_REDUCE"
            | "THIS_IMAGE" | "NUM_IMAGES" | "IMAGE_INDEX"
    )
}
