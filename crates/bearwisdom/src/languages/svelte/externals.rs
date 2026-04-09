// =============================================================================
// svelte/externals.rs — Svelte external globals
// =============================================================================

use std::collections::HashSet;

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

/// Dependency-gated framework globals for Svelte.
pub(crate) fn framework_globals(deps: &HashSet<String>) -> Vec<&'static str> {
    let mut globals = Vec::new();

    // Inherit JS/TS framework globals (test runners, i18n, etc.)
    globals.extend(crate::languages::typescript::externals::framework_globals(deps));

    // SvelteKit type exports (load, action, request handler types that appear
    // as bare identifiers in route files).
    if deps.contains("@sveltejs/kit") {
        globals.extend(SVELTEKIT_TYPE_GLOBALS);
    }

    globals
}

const SVELTEKIT_TYPE_GLOBALS: &[&str] = &[
    "PageLoad",
    "PageData",
    "PageServerLoad",
    "PageServerData",
    "LayoutLoad",
    "LayoutData",
    "LayoutServerLoad",
    "LayoutServerData",
    "Actions",
    "ActionData",
    "RequestHandler",
    "EntryGenerator",
    "ParamMatcher",
];
