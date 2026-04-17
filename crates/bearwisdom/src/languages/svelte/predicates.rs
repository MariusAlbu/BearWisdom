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

/// Check whether a name is a Svelte runtime or SvelteKit builtin that will
/// never appear in the project symbol index.
pub(super) fn is_svelte_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);

    // Delegate JS/TS runtime globals first.
    if crate::languages::typescript::predicates::is_js_runtime_global(root) {
        return true;
    }

    matches!(
        root,
        // ── Svelte 4 lifecycle / context ────────────────────────────────────
        "onMount"
            | "onDestroy"
            | "beforeUpdate"
            | "afterUpdate"
            | "tick"
            | "createEventDispatcher"
            | "setContext"
            | "getContext"
            | "hasContext"
            | "getAllContexts"
            // ── Svelte 5 runes ───────────────────────────────────────────────
            | "$state"
            | "$derived"
            | "$effect"
            | "$props"
            | "$bindable"
            | "$inspect"
            | "$host"
            // ── Svelte stores ────────────────────────────────────────────────
            | "writable"
            | "readable"
            | "derived"
            | "get"
            | "readonly"
            // ── SvelteKit navigation ─────────────────────────────────────────
            | "goto"
            | "invalidate"
            | "invalidateAll"
            | "prefetch"
            | "prefetchRoutes"
            | "beforeNavigate"
            | "afterNavigate"
            | "onNavigate"
            | "pushState"
            | "replaceState"
            // ── SvelteKit environment/store singletons ───────────────────────
            | "page"
            | "navigating"
            | "updated"
            | "browser"
            | "building"
            | "dev"
            | "version"
            // ── SvelteKit module virtual imports ─────────────────────────────
            // These appear as identifiers after `import { ... } from "$app/..."`.
            | "enhance"
            | "applyAction"
            | "deserialize"
            | "base"
            | "assets"
            | "resolveRoute"
            | "env"
            // ── Svelte template globals ──────────────────────────────────────
            | "$page"
            | "$navigating"
            | "$updated"
            | "$$props"
            | "$$restProps"
            | "$$slots"
    )
}
