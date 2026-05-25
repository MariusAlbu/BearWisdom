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

/// Stdlib crate roots that always classify as external at the resolver's
/// "first ::-segment" decision point. `core` and `alloc` are re-exported
/// through `std` in most builds, but also valid as direct imports in no_std
/// crates. `test` is the unstable libtest crate (`extern crate test;`); it
/// ships with rustc and provides `Bencher` for `#[bench]` functions.
/// `proc_macro` is the proc-macro authoring API. Callers treat any of these
/// as the "std" ecosystem for origin classification purposes.
pub(crate) const STDLIB_CRATES: &[&str] = &["std", "core", "alloc", "test", "proc_macro"];

/// Names brought into scope by the implicit `std::prelude::v1` import in
/// every Rust module. The compiler imports these unconditionally so the
/// resolver must too — otherwise bare `Vec`, `Some`, `format!`, `println!`
/// etc. land as unresolved even when the `rust-stdlib` walker has indexed
/// `core`/`alloc`/`std`.
///
/// Source: <https://doc.rust-lang.org/std/prelude/index.html> (v1 + Rust
/// 2024 additions: `Future`, `IntoFuture`).
pub(crate) const PRELUDE_NAMES: &[&str] = &[
    // Types
    "Box", "String", "Vec",
    // Enums + variant brought-in names
    "Option", "Some", "None",
    "Result", "Ok", "Err",
    // Marker / function-shape traits
    "Copy", "Send", "Sized", "Sync", "Unpin",
    "Drop",
    "Fn", "FnMut", "FnOnce",
    // Conversion / borrowing traits
    "AsMut", "AsRef", "From", "Into",
    "TryFrom", "TryInto",
    "ToOwned",
    // Equality / ordering / hash / clone
    "Clone", "Eq", "Ord", "PartialEq", "PartialOrd", "Hash",
    // Formatting / serialization / default
    "Debug", "Display", "Default", "ToString",
    // Iteration
    "DoubleEndedIterator", "ExactSizeIterator", "Extend",
    "IntoIterator", "Iterator", "FromIterator",
    // Async (2024 prelude)
    "Future", "IntoFuture",
    // Macros (callable bare without `use`)
    "assert", "assert_eq", "assert_ne",
    "debug_assert", "debug_assert_eq", "debug_assert_ne",
    "cfg", "column", "concat", "dbg",
    "env", "eprint", "eprintln",
    "file", "format",
    "include", "include_bytes", "include_str",
    "line", "matches", "module_path",
    "option_env", "panic", "print", "println",
    "stringify", "thread_local",
    "todo", "unimplemented", "unreachable",
    "vec", "write", "writeln",
];

/// True for virtual file paths produced by the `rust-stdlib` ecosystem
/// walker — `<prefix>/rustlib/src/rust/library/<crate>/...`.
pub(crate) fn is_rust_stdlib_path(file_path: &str) -> bool {
    file_path.contains("/rustlib/src/rust/library/")
        || file_path.contains("\\rustlib\\src\\rust\\library\\")
}
