// =============================================================================
// nim/keywords.rs — Nim primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Nim.
pub(crate) const KEYWORDS: &[&str] = &[
    // keywords / flow
    "echo", "var", "let", "const", "proc", "func", "method", "iterator",
    "template", "macro", "type", "object", "enum", "tuple", "ref", "ptr",
    "distinct", "concept", "converter",
    "import", "include", "from", "export",
    "when", "case", "of", "if", "elif", "else",
    "while", "for", "in", "notin", "is", "isnot",
    "not", "and", "or", "xor", "shl", "shr", "div", "mod",
    "nil", "true", "false",
    "discard", "return", "yield", "break", "continue",
    "result", "assert", "doAssert", "debugEcho", "quit",
    "stdin", "stdout", "stderr",
    // primitive types
    "int", "int8", "int16", "int32", "int64",
    "uint", "uint8", "uint16", "uint32", "uint64",
    "float", "float32", "float64",
    "bool", "char", "string", "cstring", "pointer",
    "Natural", "Positive",
    // collection types
    "seq", "array", "openArray", "set",
    "HashSet", "Table", "OrderedTable", "CountTable", "Deque",
    // built-in functions
    "len", "high", "low", "inc", "dec", "succ", "pred",
    "abs", "min", "max", "clamp",
    "add", "del", "delete", "insert", "pop",
    "contains", "find", "count",
    "sort", "sorted", "reverse", "reversed",
    "map", "filter", "foldl", "apply",
    "mapIt", "filterIt", "anyIt", "allIt",
    "toSeq", "pairs", "items", "mitems", "mpairs",
    "newSeq", "newString", "newStringOfCap",
    "repr",
    // operators
    "$", "&", "@",
    "==", "!=", "<", ">", "<=", ">=",
    "+", "-", "*", "/", "%", "^",
    "..", "..<",
    "addr", "unsafeAddr",
    "sizeof", "alignof", "offsetof", "typeof",
    // compile-time / meta
    "compileOption", "defined", "declared",
    "gorge", "staticRead", "staticExec", "slurp",
    "currentSourcePath", "instantiationInfo",
    // standard library modules (used as identifiers)
    "strutils", "strformat", "sequtils", "tables", "sets",
    "os", "osproc", "json", "parseopt",
    "httpclient", "asyncdispatch", "asynchttpserver",
    "logging", "unittest", "times", "math", "algorithm",
    "sugar", "options", "uri", "base64", "md5",
    "terminal", "parseutils", "nativesockets", "net",
    "streams", "threadpool", "locks", "macros", "typetraits",
    "jester", "karax", "nimpy",
];
