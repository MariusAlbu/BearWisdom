// =============================================================================
// bash/predicates.rs — Bash builtin and helper predicates
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
///
/// Single source of truth — delegates to the `KEYWORDS` table in
/// `bash/keywords.rs`. Previously this was a duplicate `matches!`
/// arm that drifted: `tput`, `history`, `basename`, `du`, etc. were
/// listed in `KEYWORDS` but absent here, so the resolver classified
/// every shell-script call to those commands as unresolved despite
/// the rest of the plugin treating them as built-ins.
pub(super) fn is_bash_builtin(name: &str) -> bool {
    super::keywords::KEYWORDS.contains(&name)
}
