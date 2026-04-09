/// Lua standard library globals, Neovim API roots, and LuaJIT/Love2D modules
/// that are never defined inside a project.
pub(crate) const EXTERNALS: &[&str] = &[
    // -------------------------------------------------------------------------
    // Lua 5.x global functions
    // -------------------------------------------------------------------------
    "assert", "collectgarbage", "dofile", "error", "getmetatable",
    "setmetatable", "ipairs", "pairs", "load", "loadfile", "next",
    "pcall", "xpcall", "print", "rawequal", "rawget", "rawlen", "rawset",
    "select", "tonumber", "tostring", "type", "require", "unpack",
    // -------------------------------------------------------------------------
    // Standard library modules
    // -------------------------------------------------------------------------
    "table", "string", "math", "io", "os", "coroutine", "debug", "package", "utf8",
    // table.*
    "table.insert", "table.remove", "table.sort", "table.concat",
    "table.move", "table.pack", "table.unpack",
    // string.*
    "string.byte", "string.char", "string.find", "string.format",
    "string.gmatch", "string.gsub", "string.len", "string.lower", "string.upper",
    "string.match", "string.rep", "string.reverse", "string.sub",
    // math.*
    "math.abs", "math.ceil", "math.floor", "math.max", "math.min",
    "math.sqrt", "math.random", "math.randomseed", "math.huge", "math.pi",
    // io.*
    "io.open", "io.close", "io.read", "io.write", "io.lines",
    // os.*
    "os.clock", "os.date", "os.time", "os.execute", "os.getenv",
    // -------------------------------------------------------------------------
    // Neovim API
    // -------------------------------------------------------------------------
    "vim",
    "vim.api", "vim.fn", "vim.cmd", "vim.opt", "vim.g", "vim.b", "vim.w",
    "vim.keymap", "vim.lsp", "vim.treesitter", "vim.diagnostic",
    "vim.schedule", "vim.loop", "vim.notify",
    "vim.tbl_deep_extend", "vim.tbl_extend",
    "vim.inspect", "vim.split", "vim.trim",
    "vim.startswith", "vim.endswith",
    // -------------------------------------------------------------------------
    // Love2D
    // -------------------------------------------------------------------------
    "love", "love.graphics", "love.audio", "love.physics",
    "love.keyboard", "love.mouse", "love.event", "love.filesystem",
    "love.window", "love.timer",
    // -------------------------------------------------------------------------
    // LuaJIT
    // -------------------------------------------------------------------------
    "ffi", "bit", "jit",
];
