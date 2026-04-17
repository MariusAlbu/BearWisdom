// =============================================================================
// proto/keywords.rs — Protocol Buffers primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Protocol Buffers.
pub(crate) const KEYWORDS: &[&str] = &[
    // scalar types
    "string", "int32", "int64", "uint32", "uint64",
    "sint32", "sint64", "fixed32", "fixed64",
    "sfixed32", "sfixed64", "float", "double", "bool", "bytes",
    // structural keywords
    "enum", "message", "service", "rpc", "returns", "stream",
    "option", "import", "package", "syntax",
    "required", "optional", "repeated", "map", "oneof",
    "reserved", "extensions", "extend", "group",
    // literals
    "true", "false", "default", "max", "to", "weak", "public",
    // google.protobuf well-known types
    "google.protobuf.Timestamp", "google.protobuf.Duration",
    "google.protobuf.Empty", "google.protobuf.Any",
    "google.protobuf.Struct", "google.protobuf.Value",
    "google.protobuf.ListValue", "google.protobuf.FieldMask",
    "google.protobuf.BoolValue", "google.protobuf.Int32Value",
    "google.protobuf.Int64Value", "google.protobuf.UInt32Value",
    "google.protobuf.UInt64Value", "google.protobuf.FloatValue",
    "google.protobuf.DoubleValue", "google.protobuf.StringValue",
    "google.protobuf.BytesValue", "google.protobuf.NullValue",
    "google.protobuf.FileOptions", "google.protobuf.MessageOptions",
    "google.protobuf.FieldOptions", "google.protobuf.ServiceOptions",
    "google.protobuf.MethodOptions", "google.protobuf.EnumOptions",
    "google.protobuf.EnumValueOptions", "google.protobuf.OneofOptions",
    "google.protobuf.ExtensionRangeOptions",
    // google.api
    "google.api.http", "google.api.HttpRule",
    "google.api.field_behavior", "google.api.resource",
    "google.api.resource_reference",
    // google.longrunning
    "google.longrunning.Operation", "google.longrunning.Operations",
    // google.rpc
    "google.rpc.Status",
    // google.type
    "google.type.Date", "google.type.TimeOfDay",
    "google.type.LatLng", "google.type.Money",
    "google.type.Color", "google.type.PostalAddress",
    "google.type.PhoneNumber",
    // google.shopping
    "google.shopping.type.Price",
];
