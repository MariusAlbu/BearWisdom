// =============================================================================
// rust_lang/keywords.rs — Rust primitive types
// =============================================================================

/// Primitive and built-in type names for Rust.
/// Stdlib types (Vec, HashMap, Option, Result, Arc, Box, etc.) are NOT listed
/// here — they come from the RustStdlib ecosystem as indexed external symbols.
/// Only language-intrinsic primitives remain: numeric/bool/char/str keywords,
/// generic type parameter conventions, and function-trait compiler intrinsics.
pub(crate) const KEYWORDS: &[&str] = &[
    // Numeric primitives
    "i8", "i16", "i32", "i64", "i128",
    "u8", "u16", "u32", "u64", "u128",
    "f32", "f64", "bool", "char", "str", "usize", "isize",
    // Self — keyword, not a stdlib type
    "Self",
    // Generic type parameters
    "T", "U", "K", "V", "E", "R", "S", "P", "A", "B", "C", "D", "N", "M",
    // Function traits — compiler intrinsics, no indexable source
    "Fn", "FnMut", "FnOnce",
];
