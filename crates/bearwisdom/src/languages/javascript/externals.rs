// =============================================================================
// javascript/externals.rs — JavaScript external globals
// =============================================================================

/// Runtime globals always external for JavaScript (browser + Node.js).
///
/// These supplement the TypeScript EXTERNALS list with Node.js-specific
/// identifiers that TS doesn't need to special-case.
pub(crate) const EXTERNALS: &[&str] = &[
    // Browser / Universal globals
    "console",
    "setTimeout",
    "setInterval",
    "clearTimeout",
    "clearInterval",
    "setImmediate",
    "clearImmediate",
    "queueMicrotask",
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
    "XMLHttpRequest",
    // Node.js globals
    "process",
    "require",
    "module",
    "exports",
    "__dirname",
    "__filename",
    "global",
    "globalThis",
    "Buffer",
    // -----------------------------------------------------------------
    // DOM Element / Event / Node API — method names that appear as bare
    // last-segment targets after A.3. These are universal browser APIs
    // that projects never redefine, so unconditional classification is
    // safe (a `setAttribute` call is always DOM).
    // -----------------------------------------------------------------
    // DOM traversal / selection
    "querySelector",
    "querySelectorAll",
    "getElementById",
    "getElementsByTagName",
    "getElementsByClassName",
    "getElementsByName",
    "closest",
    "matches",
    // Attribute / content
    "getAttribute",
    "setAttribute",
    "removeAttribute",
    "hasAttribute",
    "getAttributeNode",
    "setAttributeNS",
    "getAttributeNS",
    "hasAttributes",
    "toggleAttribute",
    "innerHTML",
    "outerHTML",
    "innerText",
    "textContent",
    "nodeValue",
    // Tree mutation
    "appendChild",
    "removeChild",
    "replaceChild",
    "insertBefore",
    "insertAdjacentHTML",
    "insertAdjacentText",
    "insertAdjacentElement",
    "cloneNode",
    "normalize",
    "contains",
    // Events
    "addEventListener",
    "removeEventListener",
    "dispatchEvent",
    "preventDefault",
    "stopPropagation",
    "stopImmediatePropagation",
    // Focus / selection / clipboard
    "scrollTo",
    "scrollBy",
    "scrollIntoView",
    // Form / input helpers
    "checkValidity",
    "reportValidity",
    "setCustomValidity",
    // CSSOM / geometry
    "getBoundingClientRect",
    "getClientRects",
    "getComputedStyle",
    // Function prototype — apply/call are shared by every function value
    "bind",
    "apply",
    "Reflect",
    // Object / Array / Promise / Symbol static builder methods that often
    // appear as `Object.defineProperty`, `Promise.resolve`, etc. — last-
    // segment form after A.3
    "defineProperty",
    "defineProperties",
    "getOwnPropertyDescriptor",
    "getOwnPropertyNames",
    "getOwnPropertySymbols",
    "getPrototypeOf",
    "setPrototypeOf",
    "assign",
    "freeze",
    "isFrozen",
    "seal",
    "isSealed",
    "entries",
    "fromEntries",
    "keys",
    "values",
    "hasOwn",
    "create",
    "is",
];

