// =============================================================================
// fsharp/primitives.rs — F# primitive types
// =============================================================================

/// Primitive and built-in type names for F#.
pub(crate) const PRIMITIVES: &[&str] = &[
    "int", "int8", "int16", "int32", "int64",
    "uint8", "uint16", "uint32", "uint64",
    "float", "float32", "double", "decimal",
    "bool", "char", "string", "unit", "obj", "byte",
    "sbyte", "nativeint", "unativeint", "bigint",
    "exn", "void",
];
