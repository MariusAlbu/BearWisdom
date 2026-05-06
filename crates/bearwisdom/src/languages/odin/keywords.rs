// =============================================================================
// odin/keywords.rs — Odin primitive types + built-in / stdlib procedures
//
// No Phase 5 walker for Odin's stdlib (no source-distribution package
// walker today), so the runtime/stdlib procedure names that have no
// walkable source live here as the engine's primitive set.
// =============================================================================

pub(crate) const KEYWORDS: &[&str] = &[
    // primitive types
    "bool", "b8", "b16", "b32", "b64",
    "int", "i8", "i16", "i32", "i64", "i128",
    "uint", "u8", "u16", "u32", "u64", "u128",
    "uintptr", "rawptr",
    "f16", "f32", "f64",
    "complex32", "complex64", "complex128",
    "quaternion64", "quaternion128", "quaternion256",
    "string", "cstring", "rune", "byte",
    "typeid", "any", "void",
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
    // built-in procedures (language spec)
    "len", "cap", "size_of", "align_of", "offset_of",
    "type_of", "typeid_of", "type_info_of",
    "make", "new", "free", "free_all", "delete",
    "append", "inject_at", "assign_at",
    "remove", "remove_range",
    "clear", "reserve", "resize",
    "copy", "unordered_remove", "ordered_remove",
    "pop", "pop_front", "push", "peek",
    "incl", "excl",
    "min", "max", "abs", "clamp", "expand_values",
    "assert", "panic", "unimplemented", "unreachable",
    "print", "println", "printf",
    "eprint", "eprintln", "eprintf",
    "cast", "auto_cast", "transmute",
    "swizzle", "complex", "real", "imag", "conj",
    "jmag", "kmag", "quaternion",
    "clone", "destroy", "close",
    // context / allocator
    "context",
    // fmt package
    "aprintf", "tprintf", "sbprintf", "bprintf", "panicf", "assertf",
    // mem package
    "alloc", "alloc_bytes", "zero", "zero_item",
    "copy_non_overlapping", "set",
    "default_allocator", "nil_allocator", "panic_allocator",
    // strings package
    "clone_to_cstring", "builder_make", "builder_reset",
    "builder_destroy", "builder_to_string",
    "concatenate", "join", "contains", "has_prefix", "has_suffix",
    "split", "split_multi", "fields", "trim_space", "trim",
    "to_upper", "to_lower", "index", "last_index", "count", "replace",
    // math package
    "sqrt", "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
    "floor", "ceil", "round", "trunc", "log", "log2", "log10",
    "exp", "exp2", "pow", "mod", "remainder", "hypot",
    // os package
    "read_entire_file", "write_entire_file",
    "open", "read", "write", "seek",
    "rename", "getenv", "setenv",
    "exit", "get_current_directory", "change_directory",
    // slice package
    "sort", "sort_by", "filter", "map", "reduce", "reverse",
    // runtime package
    "default_temp_allocator", "heap_allocator",
    // core packages (commonly referenced as imports)
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
