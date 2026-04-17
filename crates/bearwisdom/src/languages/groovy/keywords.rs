// =============================================================================
// groovy/keywords.rs — Groovy primitive types
// =============================================================================

/// Primitive and built-in type names for Groovy.
/// String/Object/List/Map/BigDecimal/BigInteger are now indexed as
/// external symbols by JdkSrc + GroovyStdlib. Only keyword primitives
/// and `def` (Groovy's dynamic type keyword) remain.
pub(crate) const KEYWORDS: &[&str] = &[
    // Keyword primitives
    "void", "boolean", "byte", "char", "short", "int", "long", "float", "double",
    // Groovy-specific keyword
    "def",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S",
    // From former builtin_type_names:
    "String", "Object", "List", "Map", "GString", "BigDecimal", "BigInteger",
];
