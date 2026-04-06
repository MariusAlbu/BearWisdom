// =============================================================================
// perl/builtins.rs — Perl builtin and helper predicates
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

/// Perl built-in functions that are never in the project index.
pub(super) fn is_perl_builtin(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "say"
            | "die"
            | "warn"
            | "chomp"
            | "chop"
            | "split"
            | "join"
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "map"
            | "grep"
            | "sort"
            | "reverse"
            | "keys"
            | "values"
            | "exists"
            | "delete"
            | "defined"
            | "ref"
            | "bless"
            | "open"
            | "close"
            | "read"
            | "write"
            | "seek"
            | "tell"
            | "eof"
            | "length"
            | "substr"
            | "index"
            | "rindex"
            | "sprintf"
            | "printf"
    )
}
