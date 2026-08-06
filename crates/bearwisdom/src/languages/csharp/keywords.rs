// =============================================================================
// csharp/keywords.rs — C# primitive types
// =============================================================================

/// Primitive and built-in type names for C#.
/// Includes keyword aliases, special types, and native integer types.
pub(crate) const KEYWORDS: &[&str] = &[
    "int", "long", "float", "double", "bool", "char", "byte", "string", "object", "void",
    "decimal", "dynamic", "short", "ushort", "uint", "ulong", "sbyte", "nint", "nuint",
    // From former builtin_type_names:
    "var",
];

/// The BCL type a C# predefined-type keyword aliases (`string` →
/// `System.String`). A static-member call on the keyword form
/// (`string.IsNullOrWhiteSpace(…)`) roots the chain on this type.
pub(crate) fn bcl_type_for_keyword(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        "string" => "System.String",
        "bool" => "System.Boolean",
        "byte" => "System.Byte",
        "sbyte" => "System.SByte",
        "char" => "System.Char",
        "decimal" => "System.Decimal",
        "double" => "System.Double",
        "float" => "System.Single",
        "int" => "System.Int32",
        "uint" => "System.UInt32",
        "long" => "System.Int64",
        "ulong" => "System.UInt64",
        "short" => "System.Int16",
        "ushort" => "System.UInt16",
        "object" => "System.Object",
        _ => return None,
    })
}
