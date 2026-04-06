// =============================================================================
// typescript/primitives.rs — TypeScript/JavaScript primitive types
// =============================================================================

/// Primitive and built-in type names for TypeScript and JavaScript.
/// Includes keyword types, wrapper objects, common globals, and generic
/// type parameter names.
pub(crate) const PRIMITIVES: &[&str] = &[
    // Keyword types
    "string", "number", "boolean", "void", "null", "undefined", "any", "never",
    "object", "symbol", "bigint", "unknown",
    // Wrapper / global objects
    "String", "Number", "Boolean", "Object", "Array", "Function", "Symbol",
    "RegExp", "Date", "Error", "Promise", "Map", "Set",
    // Generic type parameters
    "T", "U", "K", "V", "P", "R", "S", "E",
];
