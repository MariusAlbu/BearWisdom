// =============================================================================
// java/primitives.rs — Java primitive types
// =============================================================================

/// Primitive and built-in type names for Java.
/// java.lang / java.util / java.io / java.util.function / java.util.logging
/// types are now indexed as external symbols by the JdkSrc ecosystem —
/// they come from $JAVA_HOME/lib/src.zip. Only keyword primitives and
/// generic type parameter conventions remain.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Keyword primitives — no indexable source
    "int", "long", "float", "double", "boolean", "char", "byte", "short", "void",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
    // From former builtin_type_names:
    "var",
];
