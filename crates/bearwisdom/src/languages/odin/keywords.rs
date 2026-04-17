// =============================================================================
// odin/keywords.rs — Odin primitive and built-in types
// =============================================================================

/// Primitive and built-in type/function names for Odin.
pub(crate) const KEYWORDS: &[&str] = &[
    // built-in procedures
    "len", "cap", "size_of", "align_of", "type_of", "typeid_of",
    "offset_of", "type_info_of",
    "swizzle", "complex", "real", "imag", "conj",
    "jmag", "kmag", "quaternion",
    "min", "max", "abs", "clamp", "expand_values",
    "assert", "panic", "unimplemented", "unreachable",
    "new", "make", "delete", "free", "free_all",
    "append", "clear", "reserve", "resize",
    "pop", "pop_front", "remove", "remove_range",
    "inject_at", "assign_at",
    "copy", "clone", "destroy", "close",
    "transmute", "auto_cast", "cast",
    // context
    "context",
    // primitive types
    "rawptr", "typeid", "any",
    "cstring", "string", "rune", "bool",
    "b8", "b16", "b32", "b64",
    "i8", "i16", "i32", "i64", "i128",
    "u8", "u16", "u32", "u64", "u128",
    "f16", "f32", "f64",
    "int", "uint", "uintptr", "byte",
    // endian-specific integers
    "i16le", "i32le", "i64le", "i128le",
    "u16le", "u32le", "u64le", "u128le",
    "i16be", "i32be", "i64be", "i128be",
    "u16be", "u32be", "u64be", "u128be",
    "f16le", "f32le", "f64le",
    "f16be", "f32be", "f64be",
    // literals
    "nil", "true", "false",
    // misc types
    "Rune", "BigInt", "TokenPos",
    // compiler directives
    "#force_inline",
    // core packages (commonly referenced)
    "base:runtime", "base:intrinsics", "core:testing",
    "core:fmt", "core:os", "core:strings", "core:math",
    "core:mem", "core:log", "core:slice", "core:io",
    "core:net", "core:sync", "core:thread", "core:time",
    "core:unicode", "core:encoding", "core:crypto",
    "core:hash", "core:compress", "core:container",
    "core:image", "core:reflect", "core:sys",
    "core:path",
    // AST helpers
    "case_ast_node", "ast_node",
];
