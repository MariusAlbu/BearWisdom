/// Browser runtime globals for Svelte component files.
/// Svelte/SvelteKit API symbols come from node_modules/svelte/ and are
/// indexed by the TypeScript externals locator.
///
/// Svelte 5 runes ($state, $derived, $effect) are compiler transforms, not
/// runtime functions — they don't exist in node_modules either. Keep them
/// here as they are truly unindexable.
pub(crate) const EXTERNALS: &[&str] = &[
    "console", "setTimeout", "setInterval", "clearTimeout", "clearInterval",
    "fetch", "URL", "URLSearchParams", "AbortController",
    "TextEncoder", "TextDecoder", "structuredClone", "atob", "btoa",
    "crypto", "performance",
    "document", "window", "navigator", "location", "history",
    "localStorage", "sessionStorage", "globalThis",
    // Svelte 5 runes — compiler transforms, no source on disk
    "$state", "$derived", "$effect", "$props", "$bindable", "$inspect", "$host",
    // Svelte template magic variables
    "$$props", "$$restProps", "$$slots",
];
