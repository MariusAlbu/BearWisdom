// =============================================================================
// scss/predicates.rs — SCSS builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Edge-kind / symbol-kind compatibility for SCSS.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Sass built-in module names used with `@use 'sass:math'` etc.
/// These are never project-defined — they come from the Sass runtime.
pub(crate) fn is_sass_builtin_module(path: &str) -> bool {
    // Match both "sass:math" and bare "math" for the module name segment.
    let stem = path.strip_prefix("sass:").unwrap_or(path);
    matches!(
        stem,
        "math" | "string" | "color" | "list" | "map" | "selector" | "meta"
    )
}


/// Returns true when a SCSS module path should be skipped during resolution —
/// either an internal CSS hint or a built-in Sass module (@use "sass:math" etc.).
pub(crate) fn is_scss_skippable_module(module: &str) -> bool {
    module == super::extract::SCSS_CSS_FN_HINT || is_sass_builtin_module(module)
}

