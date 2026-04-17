// =============================================================================
// csharp/primitives.rs — C# primitive types
// =============================================================================

/// Primitive and built-in type names for C#.
/// Includes keyword aliases, special types, and native integer types.
pub(crate) const PRIMITIVES: &[&str] = &[
    "int", "long", "float", "double", "bool", "char", "byte", "string", "object",
    "void", "decimal", "dynamic", "short", "ushort", "uint", "ulong", "sbyte",
    "nint", "nuint",
    // From former builtin_type_names:
    "var",
];
