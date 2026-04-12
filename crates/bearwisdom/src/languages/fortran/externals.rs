/// Runtime globals always external for Fortran.
/// Covers: intrinsic procedures, inquiry functions, mathematical builtins,
/// character/string intrinsics, bit manipulation, coarray, and C interop.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── Numeric / mathematical intrinsics ────────────────────────────────────
    "abs", "aint", "anint", "ceiling", "floor", "nint", "round",
    "mod", "modulo", "sign", "dim", "dprod",
    "max", "min", "max0", "max1", "min0", "min1",
    "sqrt", "exp", "log", "log10",
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "sinh", "cosh", "tanh", "asinh", "acosh", "atanh",
    "hypot",
    "bessel_j0", "bessel_j1", "bessel_jn",
    "bessel_y0", "bessel_y1", "bessel_yn",
    "erf", "erfc", "erfc_scaled", "gamma", "log_gamma",
    "norm2",
    // Complex-specific
    "aimag", "conjg", "real", "cmplx",

    // ── Type conversion / kind inquiry ───────────────────────────────────────
    "int", "real", "dble", "cmplx", "ichar", "char",
    "ibits", "ibset", "ibclr", "btest", "ieor", "ior", "iand", "not",
    "shifta", "shiftl", "shiftr", "ishft", "ishftc",
    "kind", "selected_real_kind", "selected_int_kind", "selected_char_kind",
    "range", "precision", "radix", "digits", "epsilon", "huge", "tiny",
    "minexponent", "maxexponent",
    "storage_size",
    "new_line",
    // ISO_FORTRAN_ENV kinds
    "int8", "int16", "int32", "int64",
    "real32", "real64", "real128",

    // ── Inquiry functions ────────────────────────────────────────────────────
    "present", "allocated", "associated", "nullify",
    "size", "shape", "lbound", "ubound", "rank",
    "len", "len_trim",
    "c_sizeof",

    // ── Character / string intrinsics ────────────────────────────────────────
    "trim", "adjustl", "adjustr",
    "index", "scan", "verify",
    "repeat", "achar", "iachar",
    "char", "ichar",

    // ── Array intrinsics ─────────────────────────────────────────────────────
    "sum", "product", "count", "any", "all",
    "maxval", "minval", "maxloc", "minloc", "findloc",
    "dot_product", "matmul", "transpose",
    "spread", "pack", "unpack", "reshape",
    "cshift", "eoshift", "merge",
    "parity",

    // ── Bit intrinsics ───────────────────────────────────────────────────────
    "poppar", "popcnt", "leadz", "trailz",
    "maskl", "maskr",
    "dshiftl", "dshiftr",
    "merge_bits",
    "bge", "bgt", "ble", "blt",
    "bit_size",

    // ── I/O and system ───────────────────────────────────────────────────────
    "write", "read", "open", "close", "flush", "rewind", "backspace",
    "inquire", "print",
    "stop", "error_stop", "exit",
    "system_clock", "date_and_time", "cpu_time",
    "random_number", "random_seed",
    "get_command", "get_command_argument", "get_environment_variable",
    "command_argument_count", "execute_command_line",

    // ── Pointer / memory ─────────────────────────────────────────────────────
    "move_alloc", "c_loc", "c_funloc", "c_f_pointer", "c_f_procpointer",
    "c_associated",
    "is_contiguous",

    // ── Coarray / parallel intrinsics ────────────────────────────────────────
    "co_sum", "co_broadcast", "co_max", "co_min", "co_reduce",
    "this_image", "num_images", "image_index",
    "lcobound", "ucobound",
    "sync_all", "sync_images", "sync_memory", "sync_team",
    "critical", "end_critical",
    "fail_image",
    "event_post", "event_wait",
    "lock", "unlock",

    // ── IEEE arithmetic (ieee_intrinsics) ────────────────────────────────────
    "ieee_is_nan", "ieee_is_finite", "ieee_is_negative", "ieee_is_normal",
    "ieee_class", "ieee_value", "ieee_set_flag", "ieee_get_flag",
    "ieee_support_nan", "ieee_support_inf", "ieee_support_rounding",

    // ── OpenMP runtime (omp_lib) ─────────────────────────────────────────────
    "omp_get_num_threads", "omp_get_thread_num",
    "omp_set_num_threads", "omp_get_max_threads",
    "omp_get_wtime", "omp_get_wtick",
    "omp_init_lock", "omp_set_lock", "omp_unset_lock", "omp_test_lock",
    "omp_destroy_lock",
    "omp_get_num_procs", "omp_in_parallel",

    // ── MPI common calls ─────────────────────────────────────────────────────
    "mpi_init", "mpi_finalize", "mpi_comm_rank", "mpi_comm_size",
    "mpi_send", "mpi_recv", "mpi_bcast", "mpi_reduce",
    "mpi_allreduce", "mpi_barrier", "mpi_abort",
    "mpi_gather", "mpi_scatter", "mpi_allgather",
    "mpi_sendrecv", "mpi_wait", "mpi_waitall", "mpi_isend", "mpi_irecv",
    "mpi_comm_world", "mpi_success",

    // ── Fortran 2023 additions ───────────────────────────────────────────────
    "selected_logical_kind",
    "real_kinds", "integer_kinds",
    "out_of_range",
];

