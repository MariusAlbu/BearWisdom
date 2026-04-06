// =============================================================================
// groovy/primitives.rs — Groovy primitive types
// =============================================================================

/// Primitive and built-in type names for Groovy.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Keyword primitives
    "void", "boolean", "byte", "char", "short", "int", "long", "float", "double",
    // Built-in types
    "def", "String", "Object", "List", "Map",
    "GString", "BigDecimal", "BigInteger",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
