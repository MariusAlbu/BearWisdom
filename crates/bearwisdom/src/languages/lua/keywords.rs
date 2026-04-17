// =============================================================================
// lua/keywords.rs — Lua primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Lua.
pub(crate) const KEYWORDS: &[&str] = &[
    // Lua globals
    "assert", "collectgarbage", "dofile", "error", "getfenv",
    "getmetatable", "ipairs", "load", "loadfile", "loadstring",
    "module", "next", "pairs", "pcall", "print",
    "rawequal", "rawget", "rawlen", "rawset", "require",
    "select", "setfenv", "setmetatable", "tonumber", "tostring",
    "type", "unpack", "xpcall",
    "_G", "_VERSION", "_ENV", "self",
    // string library
    "string.byte", "string.char", "string.dump", "string.find",
    "string.format", "string.gmatch", "string.gsub", "string.len",
    "string.lower", "string.match", "string.rep", "string.reverse",
    "string.sub", "string.upper",
    // table library
    "table.concat", "table.insert", "table.maxn", "table.move",
    "table.pack", "table.remove", "table.sort", "table.unpack",
    // math library
    "math.abs", "math.acos", "math.asin", "math.atan",
    "math.ceil", "math.cos", "math.deg", "math.exp",
    "math.floor", "math.fmod", "math.huge",
    "math.log", "math.max", "math.maxinteger",
    "math.min", "math.mininteger", "math.modf",
    "math.pi", "math.rad", "math.random", "math.randomseed",
    "math.sin", "math.sqrt", "math.tan",
    "math.tointeger", "math.type",
    // io library
    "io.close", "io.flush", "io.input", "io.lines", "io.open",
    "io.output", "io.popen", "io.read", "io.stderr",
    "io.stdin", "io.stdout", "io.tmpfile", "io.type", "io.write",
    // os library
    "os.clock", "os.date", "os.difftime", "os.execute",
    "os.exit", "os.getenv", "os.remove", "os.rename",
    "os.setlocale", "os.time", "os.tmpname",
    // debug library
    "debug.debug", "debug.gethook", "debug.getinfo", "debug.getlocal",
    "debug.getmetatable", "debug.getregistry", "debug.getupvalue",
    "debug.getuservalue", "debug.sethook", "debug.setlocal",
    "debug.setmetatable", "debug.setupvalue", "debug.setuservalue",
    "debug.traceback", "debug.upvalueid", "debug.upvaluejoin",
    // coroutine library
    "coroutine.create", "coroutine.isyieldable", "coroutine.resume",
    "coroutine.running", "coroutine.status", "coroutine.wrap", "coroutine.yield",
    // package library
    "package.config", "package.cpath", "package.loaded",
    "package.loadlib", "package.path", "package.preload",
    "package.searchers", "package.searchpath",
    // string methods (via colon syntax)
    "byte", "char", "find", "format", "gmatch", "gsub",
    "len", "lower", "match", "rep", "reverse", "sub", "upper",
    // Neovim API
    "vim.api", "vim.fn", "vim.cmd", "vim.keymap", "vim.opt",
    "vim.g", "vim.b", "vim.w", "vim.o", "vim.bo", "vim.wo", "vim.env",
    "vim.lsp", "vim.diagnostic", "vim.treesitter",
    "vim.ui", "vim.loop", "vim.schedule", "vim.defer_fn", "vim.notify",
    "vim.tbl_deep_extend", "vim.tbl_extend", "vim.tbl_contains",
    "vim.tbl_map", "vim.tbl_filter", "vim.tbl_keys", "vim.tbl_values",
    "vim.tbl_isempty", "vim.tbl_count",
    "vim.list_extend", "vim.split", "vim.trim",
    "vim.startswith", "vim.endswith",
    "vim.inspect", "vim.validate", "vim.is_callable", "vim.deepcopy",
    "vim.log", "vim.log.levels",
    // busted test framework
    "describe", "it", "before_each", "after_each", "pending",
    "spy", "stub", "mock", "if_nil",
];
