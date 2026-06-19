// =============================================================================
// svelte/builtins.rs — Svelte builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
///
/// Delegates to the TypeScript rules — Svelte's script blocks are TypeScript.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    crate::languages::typescript::predicates::kind_compatible(edge_kind, sym_kind)
}

/// Svelte 5 runes are compiler built-ins, not stores.
const SVELTE_RUNES: &[&str] = &[
    "state", "derived", "effect", "props", "bindable", "inspect", "host",
];

/// `$store` is Svelte sugar for subscribing to the store `store`; the
/// reference is semantically to `store`. Returns the bare identifier when
/// the name matches the store-ref pattern, `None` otherwise.
fn svelte_store_base(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('$')?;
    if rest.is_empty() || rest.starts_with('$') {
        return None;
    }
    let mut chars = rest.chars();
    let first_ok = chars
        .next()
        .map_or(false, |c| c.is_alphabetic() || c == '_');
    if !first_ok || !rest.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    if SVELTE_RUNES.contains(&rest) {
        return None;
    }
    Some(rest)
}

/// Strip the `$`-store prefix from ref names and chain root segments in place.
pub(crate) fn desugar_store_ref_in_place(rf: &mut crate::types::ExtractedRef) {
    if let Some(base) = svelte_store_base(&rf.target_name) {
        rf.target_name = base.to_string();
    }
    if let Some(chain) = rf.chain.as_mut() {
        if let Some(root) = chain.segments.first_mut() {
            if let Some(base) = svelte_store_base(&root.name) {
                root.name = base.to_string();
            }
        }
    }
}
