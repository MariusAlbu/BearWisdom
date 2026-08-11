// =============================================================================
// fortran/predicates.rs — Fortran builtin and helper predicates
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

/// Fortran 2018 §16 intrinsic procedures, implicitly visible everywhere with
/// no USE statement and no source file to index. Grouped by the standard's
/// own §16 subsections, plus the IEEE_ARITHMETIC module procedures (§17.11)
/// and the five ISO_C_BINDING procedures (§18.2) that are genuinely callable
/// (the module's derived types and named kind constants never appear as a
/// Calls-ref target, since the extractor doesn't emit refs for declared
/// types).
///
/// Deliberately excludes names that stdlib-shaped Fortran projects commonly
/// re-declare as a same-named generic interface extending the intrinsic
/// (`interface merge` / `interface count` / ...): `adjustl`, `adjustr`,
/// `char`, `count`, `iachar`, `ichar`, `index`, `len`, `len_trim`, `lge`,
/// `lgt`, `lle`, `llt`, `merge`, `random_seed`, `repeat`, `scan`,
/// `transpose`, `trim`, `unpack`, `verify`. `builtin_skip` runs before every
/// project-symbol lookup rung, so draining one of those would permanently
/// mask the project's own interface instead of letting the ladder bind it.
pub(super) const INTRINSIC_PROCEDURES: &[&str] = &[
    // Numeric
    "abs", "aimag", "aint", "anint", "ceiling", "conjg", "dim", "dprod", "floor", "fraction",
    "huge", "max", "min", "mod", "modulo", "nearest", "nint", "rrspacing", "scale", "sign",
    "sin", "sinh", "spacing", "sqrt", "tan", "tanh", "tiny", "trunc", "acos", "asin", "atan",
    "atan2", "cos", "cosh", "exp", "exponent", "log", "log10", "norm2",
    // Type conversion
    "cmplx", "dble", "float", "ifix", "idint", "int", "real", "transfer",
    // Kind and model inquiry
    "bit_size", "digits", "epsilon", "kind", "maxexponent", "minexponent", "precision", "radix",
    "range", "selected_int_kind", "selected_real_kind", "set_exponent",
    // Character/string
    "achar", "new_line",
    // Array
    "all", "any", "cshift", "dot_product", "eoshift", "findloc", "lbound", "matmul", "maxloc",
    "maxval", "minloc", "minval", "pack", "product", "reshape", "shape", "size", "spread",
    "sum", "ubound", "is_contiguous",
    // Bit manipulation
    "btest", "iand", "ibclr", "ibits", "ibset", "ieor", "ior", "ishft", "ishftc", "mvbits",
    "not",
    // Pointer and allocation status
    "allocated", "associated", "move_alloc", "null", "present",
    // Program and environment
    "command_argument_count", "cpu_time", "date_and_time", "execute_command_line",
    "get_command", "get_command_argument", "get_environment_variable", "random_number",
    "system_clock",
    // IEEE_ARITHMETIC module procedures
    "ieee_value", "ieee_quiet_nan", "ieee_positive_inf", "ieee_negative_inf",
    "ieee_support_inf", "ieee_support_nan", "ieee_support_halting", "ieee_support_rounding",
    "ieee_support_sqrt", "ieee_is_nan", "ieee_is_finite", "ieee_is_negative", "ieee_is_normal",
    "ieee_class", "ieee_copy_sign", "ieee_logb", "ieee_next_after", "ieee_rem", "ieee_rint",
    "ieee_scalb", "ieee_unordered", "ieee_get_flag", "ieee_get_halting_mode",
    "ieee_get_rounding_mode", "ieee_set_flag", "ieee_set_halting_mode",
    "ieee_set_rounding_mode",
    // ISO_C_BINDING procedures
    "c_loc", "c_associated", "c_f_pointer", "c_f_procpointer", "c_sizeof",
];

/// Fortran 2018 §16.10 standard intrinsic modules — ship with the compiler,
/// never resolve to project source. Checked against a USE statement's
/// module-name target the same way `INTRINSIC_PROCEDURES` is checked
/// against a call target.
///
/// Excludes `iso_fortran_env`: a project may legally declare its own module
/// under that exact name (observed in a test fixture), and draining it would
/// mask that project's own module the same way draining a shadowed
/// procedure name would.
pub(super) const INTRINSIC_MODULES: &[&str] =
    &["iso_c_binding", "ieee_arithmetic", "ieee_exceptions", "ieee_features"];

/// True when `name` is a Fortran-standard intrinsic procedure or intrinsic
/// module — a non-project construct the resolution ladder should decline
/// before attempting any project-symbol lookup. Case-folded because Fortran
/// identifiers are case-insensitive; the extractor preserves source-file
/// case in the ref's raw target text.
pub(super) fn is_fortran_intrinsic(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    INTRINSIC_PROCEDURES.contains(&lower.as_str()) || INTRINSIC_MODULES.contains(&lower.as_str())
}

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod tests;
