// =============================================================================
// zig/keywords.rs — Zig primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Zig.
pub(crate) const KEYWORDS: &[&str] = &[
    // builtin functions (@-prefixed)
    "@import", "@This", "@as", "@bitCast", "@intCast",
    "@floatCast", "@ptrCast", "@alignCast",
    "@enumFromInt", "@intFromEnum", "@intFromPtr", "@ptrFromInt",
    "@intFromBool", "@boolFromInt", "@truncate",
    "@errorName", "@errorCast", "@tagName",
    "@typeName", "@typeInfo", "@Type",
    "@sizeOf", "@alignOf", "@bitSizeOf", "@offsetOf",
    "@fieldParentPtr", "@field", "@hasField", "@hasDecl",
    "@min", "@max", "@abs",
    "@sqrt", "@log", "@log2", "@log10", "@exp", "@exp2",
    "@ceil", "@floor", "@round", "@mod", "@rem",
    "@divExact", "@divFloor", "@divTrunc", "@mulAdd",
    "@addWithOverflow", "@subWithOverflow", "@mulWithOverflow",
    "@shlExact", "@shlWithOverflow", "@shrExact",
    "@clz", "@ctz", "@popCount", "@byteSwap", "@bitReverse",
    "@atomicLoad", "@atomicStore", "@atomicRmw",
    "@cmpxchgStrong", "@cmpxchgWeak", "@fence",
    "@panic", "@compileError", "@compileLog",
    "@embedFile", "@cDefine", "@cImport", "@cInclude",
    "@call", "@memcpy", "@memset",
    "@reduce", "@select", "@shuffle", "@splat", "@Vector",
    "@prefetch",
    "@wasmMemorySize", "@wasmMemoryGrow",
    "@setAlignStack", "@setEvalBranchQuota",
    "@setFloatMode", "@setRuntimeSafety", "@setCold",
    "@src", "@returnAddress", "@frameAddress", "@breakpoint",
    // std namespace
    "std", "std.mem", "std.fmt", "std.fs", "std.io",
    "std.os", "std.net", "std.heap", "std.log", "std.math",
    "std.json", "std.testing", "std.debug", "std.crypto",
    "std.hash", "std.http", "std.time", "std.Thread",
    // std collections
    "std.ArrayList", "std.HashMap", "std.BoundedArray",
    "std.AutoHashMap", "std.StringHashMap",
    "std.ArrayListUnmanaged", "std.MultiArrayList",
    "std.SegmentedList", "std.PriorityQueue",
    // std allocators
    "std.Allocator", "std.GeneralPurposeAllocator",
    "std.FixedBufferAllocator", "std.ArenaAllocator",
    "std.page_allocator", "std.c_allocator",
    // primitive types
    "bool", "true", "false", "null", "undefined",
    "noreturn", "void", "anyerror", "anyframe", "anyopaque", "anytype",
    "comptime_int", "comptime_float", "type", "error",
    "i8", "i16", "i32", "i64", "i128",
    "u8", "u16", "u32", "u64", "u128",
    "f16", "f32", "f64", "f80", "f128",
    "usize", "isize",
    // C ABI types
    "c_char", "c_short", "c_int", "c_long", "c_longlong", "c_longdouble",
    "c_ushort", "c_uint", "c_ulong", "c_ulonglong",
    // common identifiers in std.testing / logging
    "assert", "debug", "err", "info", "warn",
    // misc std identifiers
    "add", "append", "getClient", "setHandler",
    "postError", "postNoMemory", "getVersion", "generate",
];
