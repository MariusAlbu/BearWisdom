// =============================================================================
// bash/builtins.rs — Bash builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
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

/// Bash builtins and common external commands that are never in the index.
pub(super) fn is_bash_builtin(name: &str) -> bool {
    matches!(
        name,
        "echo"
            | "printf"
            | "cd"
            | "ls"
            | "grep"
            | "sed"
            | "awk"
            | "export"
            | "eval"
            | "exec"
            | "read"
            | "test"
            | "exit"
            | "return"
            | "shift"
            | "set"
            | "unset"
            | "trap"
            | "wait"
            | "kill"
            | "local"
            | "declare"
            | "readonly"
            | "typeset"
            | "getopts"
    )
}
