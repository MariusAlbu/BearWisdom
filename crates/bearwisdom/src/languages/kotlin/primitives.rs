// =============================================================================
// kotlin/primitives.rs — Kotlin primitive types
// =============================================================================

/// Primitive and built-in type names for Kotlin.
/// Kotlin has no keyword primitives — all types are objects.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Core types
    "Int", "Long", "Float", "Double", "Boolean", "Char", "Byte", "Short",
    "String", "Unit", "Any", "Nothing", "Number",
    // Collections
    "Array", "List", "MutableList", "Map", "MutableMap", "Set", "MutableSet",
    "Pair", "Triple",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
];
