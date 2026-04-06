// =============================================================================
// go/primitives.rs — Go primitive types
// =============================================================================

/// Primitive and built-in type names for Go.
pub(crate) const PRIMITIVES: &[&str] = &[
    "int", "int8", "int16", "int32", "int64",
    "uint", "uint8", "uint16", "uint32", "uint64",
    "float32", "float64", "bool", "string", "byte", "rune",
    "error", "any", "complex64", "complex128", "uintptr",
    // Generic type parameters
    "T", "K", "V", "E",
];
