// =============================================================================
// nim/predicates.rs — Nim builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Identifiers from Nim's implicitly-imported `system` module that are language
/// spec — `magic`-implemented procs and intrinsics with no walkable Nim source.
/// Every Nim program auto-imports `system`, so a bare reference to one of these
/// is a language construct, never a project symbol. Drives the profile's
/// `builtin_skip` to decline such a reference before the strategy ladder.
///
/// Scope is the closed `system`-magic set only. Stdlib MODULE names
/// (`strutils`, `sequtils`, …) and ordinary stdlib procs are NOT here — those
/// resolve through the nim-stdlib externals path, not a language decline.
const NIM_SYSTEM_MAGICS: &[&str] = &[
    // I/O and program control intrinsics
    "echo",
    "debugEcho",
    "quit",
    "assert",
    "doAssert",
    // length / ordinal intrinsics
    "len",
    "high",
    "low",
    "succ",
    "pred",
    "ord",
    "chr",
    // value lifecycle intrinsics
    "new",
    "newSeq",
    "newString",
    "newStringOfCap",
    "reset",
    "move",
    "wasMoved",
    "addr",
    "unsafeAddr",
    // representation / introspection intrinsics
    "repr",
    "typeof",
    "sizeof",
    "alignof",
    "offsetof",
    // sequence/openArray mutation intrinsics
    "add",
    "del",
    "delete",
    "insert",
    "pop",
    "setLen",
    "swap",
    // arithmetic intrinsics
    "inc",
    "dec",
    "abs",
    "min",
    "max",
    // discard / default
    "discard",
    "default",
];

/// True when `name` is a Nim `system`-module language intrinsic
/// (see `NIM_SYSTEM_MAGICS`). Drives the profile's `builtin_skip`.
pub(super) fn is_nim_system_magic(name: &str) -> bool {
    NIM_SYSTEM_MAGICS.contains(&name)
}

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method"
                | "function"
                | "constructor"
                | "test"
                | "class"
                | "enum_member"
                | "enum"
                | "struct"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod tests;
