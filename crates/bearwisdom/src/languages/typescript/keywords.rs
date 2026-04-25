// =============================================================================
// typescript/keywords.rs — TypeScript/JavaScript primitive types
// =============================================================================

/// Primitive and built-in type names for TypeScript and JavaScript.
///
/// NOTE on what stays vs goes: the resolver's `AMBIGUITY_LIMIT = 10`
/// (see heuristic.rs) skips resolution when more than 10 kind-compatible
/// external candidates exist for a name. Trimming a name that has > 10
/// candidates in an indexed project (e.g., Array has 64 kind-compatible
/// hits across the lib.es*.d.ts/lib.dom.d.ts tree) causes it to become
/// unresolved rather than resolve via TsLibDom. Those names therefore
/// stay here as disambiguation short-circuits until the resolver learns
/// to prefer lib.es5 > lib.dom > @types/X.
pub(crate) const KEYWORDS: &[&str] = &[
    // Keyword types
    "string", "number", "boolean", "void", "null", "undefined", "any", "never",
    "object", "symbol", "bigint", "unknown",
    // Wrapper / global objects — all over AMBIGUITY_LIMIT (64-28 candidates)
    "String", "Number", "Boolean", "Object", "Array", "Function", "Symbol",
    "RegExp", "Date", "Error", "Promise", "Map", "Set",
    // JS namespace globals — appear in `Math.floor()`, `JSON.parse()` etc.
    // Used as both value and type (`typeof Math`, `JSON.stringify`).
    "Math", "JSON", "console", "globalThis", "performance",
    // Utility types that exceed AMBIGUITY_LIMIT (11-16 candidates each)
    "Partial", "Required", "Readonly", "ReturnType", "Parameters",
    "ThisType",
    // Utility types with <=10 candidates are removed — resolver handles them
    // via TsLibDom indexing: Record, Pick, Omit, Exclude, Extract,
    // NonNullable, InstanceType, ConstructorParameters, ThisParameterType,
    // OmitThisParameter, Awaited, PromiseLike, ArrayLike, Iterable,
    // Lowercase, Uppercase, Capitalize, Uncapitalize, PropertyKey,
    // PropertyDescriptor, PropertyDescriptorMap, TypedPropertyDescriptor,
    // TemplateStringsArray.
    // Error subtypes — 7-9 candidates, removed.
    // Typed arrays and buffers
    "ArrayBuffer", "SharedArrayBuffer", "DataView",
    "Uint8Array", "Uint16Array", "Uint32Array",
    "Int8Array", "Int16Array", "Int32Array",
    "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array",
    // Collections — WeakMap=21, WeakSet=23 over limit; WeakRef=8 removable
    "WeakMap", "WeakSet",
    // Browser / Node.js API types — most exceed AMBIGUITY_LIMIT (20-70)
    "Buffer", "Blob", "File", "FormData", "Headers", "Request", "Response",
    "URL", "URLSearchParams", "AbortController", "AbortSignal",
    "ReadableStream", "WritableStream", "TransformStream",
    "HTMLElement", "HTMLDivElement", "HTMLInputElement", "HTMLButtonElement",
    "HTMLImageElement", "HTMLFormElement", "HTMLAnchorElement",
    "HTMLCanvasElement", "HTMLVideoElement", "HTMLAudioElement",
    "HTMLSelectElement", "HTMLTextAreaElement", "HTMLSpanElement",
    "Element", "Node", "Document", "Event", "EventTarget",
    "MouseEvent", "KeyboardEvent", "FocusEvent", "InputEvent",
    "CustomEvent", "PointerEvent", "TouchEvent", "DragEvent",
    "IntersectionObserver", "MutationObserver", "ResizeObserver",
    "MessageEvent", "WebSocket", "Worker", "ServiceWorker",
    "Crypto", "CryptoKey", "TextEncoder", "TextDecoder",
    // Iterators / generators — most exceed limit (Iterator=35, Generator=11)
    "Iterator", "AsyncIterator", "Generator",
    // Synthetic — emitted by extractor for primitive type annotations
    "_primitive",
    // TS keyword types (appear as type_ref in extractor output)
    "typeof", "keyof", "infer", "const", "readonly", "import",
    "asserts", "is", "out", "in", "extends", "implements",
    // React/Next/Zod/fp-ts/Radix/TanStack — NPM packages, come from
    // NpmEcosystem when the project depends on them. Simple forms with
    // ≤10 kind-compatible candidates are removed; qualified forms (React.X,
    // JSX.X, z.X, E.Either, etc.) were never indexed that way and are dead
    // entries in the original primitives list.
    // Generic type parameters
    "T", "U", "K", "V", "P", "R", "S", "E", "A", "B", "O",
    // Syntactic keywords that the extractor emits as refs (receiver of
    // `super(...)` / `super.x()` call, `this` binding, `new.target`).
    // These aren't symbol references — they're language constructs — so
    // classifying them as "primitive" external suppresses spurious
    // unresolved_refs entries.
    "super", "this", "new.target", "import.meta",
    // Svelte 5 runes — compiler intrinsics callable in `<script>` blocks
    // and `.svelte.ts` files. The Svelte plugin reuses TS keywords, so
    // listing them here covers both `.svelte` host files (where pf.language
    // is "svelte") and `.svelte.ts` files (where pf.language is "typescript").
    // No import statement is needed in source — they're resolved by the
    // Svelte compiler — so they should be classified as "primitive".
    "$state", "$derived", "$effect", "$props", "$bindable", "$inspect", "$host",
    // Svelte 4 / SvelteKit specials. `$$props`, `$$restProps`, `$$slots`
    // are auto-injected in template scope; `$page`, `$navigating`, `$updated`
    // are SvelteKit auto-imported store proxies (the leading `$` is the
    // store-auto-subscribe prefix).
    "$$props", "$$restProps", "$$slots",
    "$page", "$navigating", "$updated",
];
