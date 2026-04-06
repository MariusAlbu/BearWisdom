// =============================================================================
// zig/builtins.rs — Zig builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "class"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Zig builtin functions and the std namespace always in scope.
pub(super) fn is_zig_builtin(name: &str) -> bool {
    matches!(
        name,
        "@import"
            | "@intCast"
            | "@floatCast"
            | "@as"
            | "@bitCast"
            | "@ptrCast"
            | "@alignCast"
            | "@truncate"
            | "@enumFromInt"
            | "@intFromEnum"
            | "@intFromBool"
            | "@intFromFloat"
            | "@floatFromInt"
            | "@sizeOf"
            | "@alignOf"
            | "@bitSizeOf"
            | "@typeInfo"
            | "@typeName"
            | "@TypeOf"
            | "@tagName"
            | "@fieldParentPtr"
            | "@field"
            | "@errorName"
            | "@panic"
            | "@compileError"
            | "@compileLog"
            | "std"
    )
}
