// =============================================================================
// graphql/primitives.rs — GraphQL primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for GraphQL.
pub(crate) const PRIMITIVES: &[&str] = &[
    // schema definition keywords
    "type", "input", "interface", "union", "enum", "scalar", "schema",
    "query", "mutation", "subscription", "fragment", "on",
    "directive", "extend", "implements",
    // built-in scalar types
    "String", "Int", "Float", "Boolean", "ID",
    // literals
    "null", "true", "false",
    // introspection types
    "__type", "__schema", "__typename",
    "__Type", "__Field", "__InputValue", "__EnumValue",
    "__Directive", "__Schema",
    // common custom scalars
    "Date", "DateTime", "Time",
    "JSON", "JSONObject", "Upload", "Void",
    "BigInt", "Long",
    // graphql-scalars / community scalars
    "UnsignedInt", "PositiveInt", "NonNegativeInt",
    "NegativeInt", "NonPositiveInt",
    "PositiveFloat", "NonNegativeFloat",
    "NegativeFloat", "NonPositiveFloat",
    "URL", "EmailAddress", "PhoneNumber", "PostalCode",
    "UUID", "GUID",
    "HexColorCode", "HSL", "HSLA", "RGB", "RGBA",
    "IPv4", "IPv6", "MAC", "Port", "ISBN",
    "ObjectID", "Byte", "Duration", "UtcOffset",
    "LocalDate", "LocalTime", "LocalDateTime",
    "Locale", "Currency", "CountryCode",
    "Latitude", "Longitude",
    "USCurrency", "SafeInt", "BigDecimal",
];
