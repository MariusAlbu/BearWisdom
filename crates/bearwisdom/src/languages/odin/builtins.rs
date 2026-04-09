// =============================================================================
// odin/builtins.rs — Odin builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test"),
        EdgeKind::Inherits => false, // Odin has no inheritance
        EdgeKind::Implements => false,
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Odin built-in type names and built-in procedures that should never resolve
/// to project symbols.
pub(super) fn is_odin_builtin(name: &str) -> bool {
    // Strip package qualifier for standard library calls (e.g. `fmt.println` → `println`).
    // The extractor already strips qualifiers for Calls edges, but TypeRef may retain them.
    let bare = name.rsplit('.').next().unwrap_or(name);

    matches!(
        bare,
        // --- Primitive types ---
        "bool" | "b8" | "b16" | "b32" | "b64"
            | "int" | "i8" | "i16" | "i32" | "i64" | "i128"
            | "uint" | "u8" | "u16" | "u32" | "u64" | "u128"
            | "uintptr" | "rawptr"
            | "f16" | "f32" | "f64"
            | "complex32" | "complex64" | "complex128"
            | "quaternion64" | "quaternion128" | "quaternion256"
            | "string" | "cstring" | "rune" | "byte"
            | "typeid" | "any" | "void"
            // --- Built-in procedures (language spec) ---
            | "len" | "cap" | "size_of" | "align_of" | "offset_of"
            | "type_of" | "typeid_of"
            | "make" | "new" | "free" | "delete"
            | "append" | "inject_at" | "remove" | "clear" | "resize"
            | "copy" | "unordered_remove" | "ordered_remove"
            | "pop" | "push" | "peek" | "incl" | "excl"
            | "min" | "max" | "abs" | "clamp"
            | "assert" | "panic" | "unimplemented" | "unreachable"
            | "print" | "println" | "printf" | "eprint" | "eprintln" | "eprintf"
            | "cast" | "auto_cast" | "transmute"
            // --- Context / allocator ---
            | "context"
            // --- Standard library proc names (after package qualifier stripped) ---
            // fmt package
            | "aprintf" | "tprintf" | "sbprintf" | "bprintf"
            | "panicf" | "assertf"
            // mem package
            | "alloc" | "alloc_bytes" | "zero" | "zero_item"
            | "copy_non_overlapping" | "set"
            | "default_allocator" | "nil_allocator" | "panic_allocator"
            // strings package
            | "clone" | "clone_to_cstring" | "builder_make" | "builder_reset"
            | "builder_destroy" | "builder_to_string"
            | "concatenate" | "join" | "contains" | "has_prefix" | "has_suffix"
            | "split" | "split_multi" | "fields" | "trim_space" | "trim"
            | "to_upper" | "to_lower" | "index" | "last_index" | "count" | "replace"
            // math package
            | "sqrt" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2"
            | "floor" | "ceil" | "round" | "trunc" | "log" | "log2" | "log10"
            | "exp" | "exp2" | "pow" | "mod" | "remainder" | "hypot"
            // os package
            | "read_entire_file" | "write_entire_file"
            | "open" | "close" | "read" | "write" | "seek"
            | "rename" | "getenv" | "setenv"
            | "exit" | "get_current_directory" | "change_directory"
            // slice package
            | "sort" | "sort_by" | "filter" | "map" | "reduce" | "reverse"
            // runtime package
            | "default_temp_allocator" | "heap_allocator"
    )
}
