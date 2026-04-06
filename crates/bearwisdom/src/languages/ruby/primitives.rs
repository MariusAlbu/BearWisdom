// =============================================================================
// ruby/primitives.rs — Ruby primitive types
// =============================================================================

/// Primitive and built-in type names for Ruby.
pub(crate) const PRIMITIVES: &[&str] = &[
    "Integer", "Float", "String", "Symbol", "Array", "Hash", "NilClass", "TrueClass",
    "FalseClass", "Numeric", "Object", "BasicObject", "Kernel", "Comparable",
    "Enumerable",
];
