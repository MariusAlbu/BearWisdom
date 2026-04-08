use std::collections::HashSet;

/// Runtime globals always external for Fortran.
/// Covers: intrinsic procedures, inquiry functions, mathematical builtins,
/// character/string intrinsics, and type-conversion functions.
pub(crate) const EXTERNALS: &[&str] = &[
    // --- Numeric / mathematical intrinsics ---
    "abs", "aint", "anint", "ceiling", "floor", "nint", "round",
    "mod", "modulo", "sign", "dim", "dprod",
    "max", "min", "max0", "max1", "min0", "min1",
    "sqrt", "exp", "log", "log10",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "sinh", "cosh", "tanh", "asinh", "acosh", "atanh",
    "hypot", "bessel_j0", "bessel_j1", "bessel_jn",
    "bessel_y0", "bessel_y1", "bessel_yn",
    "erf", "erfc", "erfc_scaled", "gamma", "log_gamma",
    // Complex-specific
    "aimag", "conjg", "real", "cmplx",

    // --- Type conversion / kind inquiry ---
    "int", "real", "dble", "cmplx", "ichar", "char",
    "ibits", "ibset", "ibclr", "btest", "ieor", "ior", "iand", "not",
    "shifta", "shiftl", "shiftr", "ishft", "ishftc",
    "kind", "selected_real_kind", "selected_int_kind", "selected_char_kind",
    "range", "precision", "radix", "digits", "epsilon", "huge", "tiny",
    "minexponent", "maxexponent",
    // ISO_FORTRAN_ENV kinds
    "int8", "int16", "int32", "int64",
    "real32", "real64", "real128",

    // --- Inquiry functions ---
    "present", "allocated", "associated", "nullify",
    "size", "shape", "lbound", "ubound", "rank",
    "len", "len_trim",
    "storage_size", "c_sizeof",

    // --- Character / string intrinsics ---
    "trim", "adjustl", "adjustr",
    "index", "scan", "verify",
    "repeat", "achar",
    "iachar",

    // --- Array intrinsics ---
    "sum", "product", "count", "any", "all",
    "maxval", "minval", "maxloc", "minloc",
    "dot_product", "matmul", "transpose",
    "spread", "pack", "unpack", "reshape",
    "cshift", "eoshift", "merge",

    // --- Bit intrinsics ---
    "poppar", "popcnt", "leadz", "trailz",

    // --- I/O and system ---
    "write", "read", "open", "close", "flush", "rewind", "backspace",
    "inquire", "print",
    "stop", "error_stop", "exit",
    "system_clock", "date_and_time", "cpu_time",
    "random_number", "random_seed",
    "get_command", "get_command_argument", "get_environment_variable",
    "command_argument_count",

    // --- Pointer / memory ---
    "move_alloc", "c_loc", "c_funloc", "c_f_pointer", "c_f_procpointer",

    // --- IEEE arithmetic (ieee_intrinsics) ---
    "ieee_is_nan", "ieee_is_finite", "ieee_is_negative", "ieee_is_normal",
    "ieee_class", "ieee_value", "ieee_set_flag", "ieee_get_flag",
    "ieee_support_nan", "ieee_support_inf", "ieee_support_rounding",

    // --- OpenMP runtime (omp_lib) ---
    "omp_get_num_threads", "omp_get_thread_num",
    "omp_set_num_threads", "omp_get_max_threads",
    "omp_get_wtime", "omp_get_wtick",

    // --- MPI common calls ---
    "mpi_init", "mpi_finalize", "mpi_comm_rank", "mpi_comm_size",
    "mpi_send", "mpi_recv", "mpi_bcast", "mpi_reduce",
    "mpi_allreduce", "mpi_barrier",
];

/// Dependency-gated framework globals for Fortran.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // test-drive unit testing framework
    for dep in ["test-drive", "test_drive", "testdrive"] {
        if deps.contains(dep) {
            globals.extend(TESTDRIVE_GLOBALS);
            break;
        }
    }

    // FRUIT (Fortran Unit Test Framework)
    for dep in ["FRUIT", "fruit"] {
        if deps.contains(dep) {
            globals.extend(FRUIT_GLOBALS);
            break;
        }
    }

    globals
}

const TESTDRIVE_GLOBALS: &[&str] = &[
    "check", "new_unittest", "collect_results",
    "test_failed", "error_type",
];

const FRUIT_GLOBALS: &[&str] = &[
    "assert_equals", "assert_not_equals", "assert_true", "assert_false",
    "assert_not", "assert_real_equals", "assert_complex_equals",
    "run_test_case", "fruit_summary",
];
