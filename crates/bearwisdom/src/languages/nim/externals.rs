/// Nim system module procs, operators, and stdlib module names that are always
/// external to any project.
///
/// Two distinct categories live here:
///
/// 1. **System proc names** — builtins from `system.nim` that are imported
///    implicitly and will never appear in the project's symbol table.
///
/// 2. **Stdlib module names** — the Nim extractor emits `imports` refs whose
///    name is the module identifier (e.g. `os`, `strutils`).  These never
///    resolve to a project symbol because only the module's *internal*
///    definitions are indexed, not the module name itself.  Listing them here
///    classifies those import refs as external rather than unresolved.
pub(crate) const EXTERNALS: &[&str] = &[
    // -----------------------------------------------------------------------
    // system.nim — implicitly imported procs / templates / macros
    // -----------------------------------------------------------------------
    "echo", "debugEcho",
    "len", "high", "low",
    "inc", "dec", "succ", "pred",
    "add", "del", "delete", "insert",
    "contains", "find",
    "hash", "cmp",
    "$", "repr",
    "new", "default", "reset",
    "init",
    "close",
    "assert", "doAssert", "raiseAssert",
    "typeof", "sizeof", "alignof", "offsetof",
    "ord", "chr",
    "abs", "min", "max", "clamp",
    "toFloat", "toInt", "toBiggestInt", "toBiggestFloat",
    "toU8", "toU16", "toU32",
    "swap", "copy", "copyMem", "moveMem", "zeroMem", "equalMem", "alloc", "alloc0",
    "allocShared", "allocShared0", "dealloc", "deallocShared", "realloc", "reallocShared",
    "addr", "unsafeAddr",
    "isNil", "not",
    "quit", "rawProc", "rawEnv",
    "defined", "declared", "compileOption",
    "when", "static",
    "nimvm",
    "result", "error",
    // -----------------------------------------------------------------------
    // Standard library module names (import targets)
    // -----------------------------------------------------------------------
    // System / OS
    "os", "osproc", "posix", "winlean",
    // Strings
    "strutils", "strformat", "strtabs", "strscans",
    "unicode", "encodings", "base64", "uri",
    // Collections
    "tables", "sets", "sequtils", "deques", "heapqueue",
    "critbits", "lists", "intsets",
    // Data / serialization
    "json", "xmltree", "xmlparser", "htmlparser",
    "parsecfg", "parsecsv", "parsesql",
    "marshal",
    // Hashing
    "hashes", "md5", "sha1", "sha1hashes",
    // Math / algorithms
    "math", "complex", "rationals", "random",
    "algorithm", "bitops", "endians",
    // I/O
    "io", "streams", "terminal", "logging",
    // Concurrency / async
    "asyncdispatch", "asyncfile", "asynchttpserver", "asyncnet",
    "asyncftpclient",
    "threadpool", "locks", "channels", "atomics",
    // Networking / HTTP
    "httpclient", "net", "nativesockets", "asyncstreams",
    // Time
    "times", "monotimes",
    // Testing
    "unittest",
    // Macros / meta
    "macros", "typetraits", "typeinfo", "genasts",
    "macrocache", "compiletime",
    "sugar",
    // Parsing / lexing
    "parseopt", "lexbase",
    // Misc stdlib
    "options", "with", "effecttraits",
    "rlocks", "selectors", "mimetypes",
    "pathnorm", "nimprof", "profiler",
    // nimble / package-manager internal modules (appear in nim-nimble project)
    "version", "common", "cli", "tools",
    "packageinfotypes", "packageinfo", "packageparser",
    "compat/json", "compat",
    "publish", "depends", "update", "install", "uninstall",
    "init", "develop",
];
