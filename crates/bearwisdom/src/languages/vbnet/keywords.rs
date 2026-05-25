// =============================================================================
// vbnet/keywords.rs — VB.NET primitive types
// =============================================================================

/// Primitive and built-in type names for VB.NET.
pub(crate) const KEYWORDS: &[&str] = &[
    "Boolean", "Byte", "SByte", "Char", "Decimal",
    "Double", "Single", "Integer", "UInteger",
    "Long", "ULong", "Short", "UShort",
    "String", "Object", "Date", "Void",
];

/// VB.NET operator keywords that parse like invocations — `NameOf(x)`,
/// `CType(x, T)`, `GetType(T)`, `TryCast(x, T)`, `DirectCast(x, T)`,
/// `AddressOf m` — but are language operators, not symbol calls, so they have
/// no resolvable target and must not be emitted as `Calls` refs.
pub(crate) const OPERATOR_KEYWORDS: &[&str] = &[
    "NameOf", "CType", "GetType", "TryCast", "DirectCast", "AddressOf",
];
