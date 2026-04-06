// =============================================================================
// python/primitives.rs — Python primitive types
// =============================================================================

/// Primitive and built-in type names for Python.
pub(crate) const PRIMITIVES: &[&str] = &[
    "int", "float", "str", "bool", "None", "bytes", "list", "dict", "tuple",
    "set", "type", "object", "complex", "frozenset", "memoryview", "range",
    "True", "False",
];
