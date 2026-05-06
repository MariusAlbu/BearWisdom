// =============================================================================
// lua/keywords.rs — Lua language keywords + interpreter built-ins
//
// Names that are ALWAYS in scope and are implemented inside the Lua
// interpreter (C source — not walkable as Lua source). Neovim API
// (vim.*) is handled by the nvim_runtime walker. Test frameworks
// (busted's `describe`, `it`, ...) come from luarocks-walked packages
// when declared in *.rockspec.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // Primitive type names
    "string", "number", "boolean", "nil", "table", "function",
    "thread", "userdata", "integer", "float",
    // Special globals
    "_G", "_VERSION", "_ENV", "self",
    // Language keywords / control flow
    "if", "elseif", "else", "end", "then",
    "for", "while", "repeat", "until", "do", "in",
    "break", "return", "goto",
    "local", "function", "and", "or", "not",
    "true", "false",
    // Global built-in functions (interpreter ops in C)
    "assert", "collectgarbage", "dofile", "error",
    "getfenv", "getmetatable",
    "ipairs", "load", "loadfile", "loadstring",
    "module", "next", "pairs", "pcall", "print",
    "rawequal", "rawget", "rawlen", "rawset", "require",
    "select", "setfenv", "setmetatable",
    "tonumber", "tostring", "type", "unpack", "xpcall",
    // Standard library tables (always-in-scope roots)
    "table", "math", "io", "os", "coroutine", "debug", "package", "utf8",
    // table.* (interpreter primitives)
    "table.concat", "table.insert", "table.maxn", "table.move",
    "table.pack", "table.remove", "table.sort", "table.unpack",
    // string.* (interpreter primitives)
    "string.byte", "string.char", "string.dump", "string.find",
    "string.format", "string.gmatch", "string.gsub", "string.len",
    "string.lower", "string.match", "string.pack", "string.packsize",
    "string.rep", "string.reverse", "string.sub", "string.unpack",
    "string.upper",
    // math.* (interpreter primitives)
    "math.abs", "math.acos", "math.asin", "math.atan",
    "math.ceil", "math.cos", "math.deg", "math.exp",
    "math.floor", "math.fmod", "math.huge",
    "math.log", "math.max", "math.maxinteger",
    "math.min", "math.mininteger", "math.modf",
    "math.pi", "math.rad", "math.random", "math.randomseed",
    "math.sin", "math.sqrt", "math.tan",
    "math.tointeger", "math.type", "math.ult",
    // io.* (interpreter primitives)
    "io.close", "io.flush", "io.input", "io.lines", "io.open",
    "io.output", "io.popen", "io.read", "io.stderr",
    "io.stdin", "io.stdout", "io.tmpfile", "io.type", "io.write",
    // os.* (interpreter primitives)
    "os.clock", "os.date", "os.difftime", "os.execute",
    "os.exit", "os.getenv", "os.remove", "os.rename",
    "os.setlocale", "os.time", "os.tmpname",
    // debug.* (interpreter primitives)
    "debug.debug", "debug.gethook", "debug.getinfo", "debug.getlocal",
    "debug.getmetatable", "debug.getregistry", "debug.getupvalue",
    "debug.getuservalue", "debug.sethook", "debug.setlocal",
    "debug.setmetatable", "debug.setupvalue", "debug.setuservalue",
    "debug.traceback", "debug.upvalueid", "debug.upvaluejoin",
    // coroutine.* (interpreter primitives)
    "coroutine.create", "coroutine.isyieldable", "coroutine.resume",
    "coroutine.running", "coroutine.status", "coroutine.wrap", "coroutine.yield",
    "coroutine.close",
    // package.* (interpreter primitives)
    "package.config", "package.cpath", "package.loaded",
    "package.loadlib", "package.path", "package.preload",
    "package.searchers", "package.searchpath",
    // utf8.* (interpreter primitives)
    "utf8.char", "utf8.charpattern", "utf8.codepoint",
    "utf8.codes", "utf8.len", "utf8.offset",
];
