// =============================================================================
// lua/builtins.rs — Lua builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
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

/// Lua standard library functions and globals that are never in the project index.
pub(super) fn is_lua_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "type"
            | "tostring"
            | "tonumber"
            | "error"
            | "assert"
            | "pcall"
            | "xpcall"
            | "pairs"
            | "ipairs"
            | "next"
            | "select"
            | "unpack"
            | "table"
            | "string"
            | "math"
            | "io"
            | "os"
            | "coroutine"
            | "debug"
            | "setmetatable"
            | "getmetatable"
            | "rawget"
            | "rawset"
            | "rawequal"
            | "rawlen"
            | "require"
            | "dofile"
            | "loadfile"
            | "load"
    )
}
