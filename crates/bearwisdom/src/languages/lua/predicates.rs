// =============================================================================
// lua/predicates.rs — Lua builtin and helper predicates
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
        // global functions
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
            | "require"
            | "dofile"
            | "loadfile"
            | "load"
            | "collectgarbage"
            | "getmetatable"
            | "setmetatable"
            | "rawequal"
            | "rawget"
            | "rawlen"
            | "rawset"
            // standard library modules (used as table roots)
            | "table"
            | "string"
            | "math"
            | "io"
            | "os"
            | "coroutine"
            | "debug"
            | "package"
            | "utf8"
            // table.*
            | "table.insert"
            | "table.remove"
            | "table.sort"
            | "table.concat"
            | "table.move"
            | "table.pack"
            | "table.unpack"
            // string.*
            | "string.byte"
            | "string.char"
            | "string.find"
            | "string.format"
            | "string.gmatch"
            | "string.gsub"
            | "string.len"
            | "string.lower"
            | "string.upper"
            | "string.match"
            | "string.rep"
            | "string.reverse"
            | "string.sub"
            // math.*
            | "math.abs"
            | "math.ceil"
            | "math.floor"
            | "math.max"
            | "math.min"
            | "math.sqrt"
            | "math.random"
            | "math.randomseed"
            | "math.huge"
            | "math.pi"
            // io.*
            | "io.open"
            | "io.close"
            | "io.read"
            | "io.write"
            | "io.lines"
            // os.*
            | "os.clock"
            | "os.date"
            | "os.time"
            | "os.execute"
            | "os.getenv"
            // Neovim API globals
            | "vim"
            | "vim.api"
            | "vim.fn"
            | "vim.cmd"
            | "vim.opt"
            | "vim.g"
            | "vim.b"
            | "vim.w"
            | "vim.keymap"
            | "vim.lsp"
            | "vim.treesitter"
            | "vim.diagnostic"
            | "vim.schedule"
            | "vim.loop"
            | "vim.notify"
            | "vim.tbl_deep_extend"
            | "vim.tbl_extend"
            | "vim.inspect"
            | "vim.split"
            | "vim.trim"
            | "vim.startswith"
            | "vim.endswith"
    )
}
