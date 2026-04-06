// =============================================================================
// scala/primitives.rs — Scala primitive types
// =============================================================================

/// Primitive and built-in type names for Scala.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Value types
    "Int", "Long", "Float", "Double", "Boolean", "Char", "Byte", "Short",
    "String", "Unit", "Any", "AnyRef", "AnyVal", "Nothing", "Null",
    // Option/Either variants
    "Some", "None", "Left", "Right", "Nil",
    // Collections and containers
    "List", "Vector", "Map", "Set", "Seq", "Array", "Option", "Either",
    "Try", "Success", "Failure", "Future", "Promise",
    "Tuple", "BigInt", "BigDecimal",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S", "A", "B",
];
