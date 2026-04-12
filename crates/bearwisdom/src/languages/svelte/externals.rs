// =============================================================================
// svelte/externals.rs — Svelte external globals
// =============================================================================

/// Runtime globals always external for Svelte components.
///
/// Includes the TypeScript EXTERNALS baseline plus Svelte runtime identifiers
/// that appear in `.svelte` script blocks without being project-defined.
pub(crate) const EXTERNALS: &[&str] = &[
    // ── TypeScript / browser baseline ────────────────────────────────────────
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "fetch",
    "URL",
    "URLSearchParams",
    "AbortController",
    "TextEncoder",
    "TextDecoder",
    "structuredClone",
    "atob",
    "btoa",
    "crypto",
    "performance",
    "document",
    "window",
    "navigator",
    "location",
    "history",
    "localStorage",
    "sessionStorage",
    "globalThis",
    // ── Svelte 4 lifecycle / context ─────────────────────────────────────────
    "onMount",
    "onDestroy",
    "beforeUpdate",
    "afterUpdate",
    "tick",
    "createEventDispatcher",
    "setContext",
    "getContext",
    "hasContext",
    "getAllContexts",
    // ── Svelte 5 runes ────────────────────────────────────────────────────────
    "$state",
    "$derived",
    "$effect",
    "$props",
    "$bindable",
    "$inspect",
    "$host",
    // ── Svelte stores ─────────────────────────────────────────────────────────
    "writable",
    "readable",
    "derived",
    "get",
    "readonly",
    // ── SvelteKit navigation ──────────────────────────────────────────────────
    "goto",
    "invalidate",
    "invalidateAll",
    "prefetch",
    "prefetchRoutes",
    "beforeNavigate",
    "afterNavigate",
    "onNavigate",
    "pushState",
    "replaceState",
    // ── SvelteKit store singletons ────────────────────────────────────────────
    "page",
    "navigating",
    "updated",
    // ── SvelteKit $app/* exports ──────────────────────────────────────────────
    "browser",
    "building",
    "dev",
    "version",
    "enhance",
    "applyAction",
    "deserialize",
    "base",
    "assets",
    "resolveRoute",
    "env",
    // ── Svelte template magic variables ──────────────────────────────────────
    "$page",
    "$navigating",
    "$updated",
    "$$props",
    "$$restProps",
    "$$slots",
];

