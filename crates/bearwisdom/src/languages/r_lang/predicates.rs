// =============================================================================
// r_lang/predicates.rs — R builtin predicate
// =============================================================================

/// True when `name` is an R language constant, control-flow keyword, or
/// C-implemented interpreter primitive with no walkable `.R` source. Drives
/// the profile's `builtin_skip` so such a reference declines before the
/// strategy ladder rather than binding to a same-named project symbol.
///
/// Single source of truth — the closed set lives in `keywords::KEYWORDS`.
/// Base/stats functions that DO have `.R` source resolve through the r_stdlib
/// walker and are deliberately absent from that set.
pub(super) fn is_r_builtin(name: &str) -> bool {
    super::keywords::KEYWORDS.contains(&name)
}

#[cfg(test)]
#[path = "predicates_tests.rs"]
mod tests;
