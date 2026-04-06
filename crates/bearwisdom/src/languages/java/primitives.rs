// =============================================================================
// java/primitives.rs — Java primitive types
// =============================================================================

/// Primitive and built-in type names for Java.
/// Includes keyword primitives, boxed wrappers, and generic type parameter names.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Keyword primitives
    "int", "long", "float", "double", "boolean", "char", "byte", "short", "void",
    // Boxed wrappers
    "Integer", "Long", "Float", "Double", "Boolean", "Character", "Byte", "Short",
    "String", "Object", "Void",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
