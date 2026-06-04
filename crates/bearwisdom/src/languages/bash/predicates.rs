// =============================================================================
// bash/predicates.rs — Bash builtin and helper predicates
// =============================================================================

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
