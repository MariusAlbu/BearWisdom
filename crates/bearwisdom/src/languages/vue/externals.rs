/// Browser runtime globals for Vue SFC files.
/// Vue API symbols (ref, computed, onMounted, etc.) come from node_modules/vue/
/// and are indexed by the TypeScript externals locator.
pub(crate) const EXTERNALS: &[&str] = &[
    "console", "setTimeout", "setInterval", "clearTimeout", "clearInterval",
    "fetch", "URL", "URLSearchParams", "AbortController",
    "TextEncoder", "TextDecoder", "structuredClone", "atob", "btoa",
    "crypto", "performance",
    "document", "window", "navigator", "location", "history",
    "localStorage", "sessionStorage", "globalThis",
];
