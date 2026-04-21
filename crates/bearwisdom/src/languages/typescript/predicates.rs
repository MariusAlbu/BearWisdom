// =============================================================================
// typescript/predicates.rs — TypeScript/JavaScript builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// A bare specifier is a module path that does not start with ".", "/", or a
/// drive letter — i.e., it refers to an npm package or Node.js built-in.
///
/// Examples:
///   - `"react"` → bare (external)
///   - `"@tanstack/react-query"` → bare (external)
///   - `"node:fs"` → bare (external)
///   - `"./utils"` → relative (internal)
///   - `"../shared/types"` → relative (internal)
///   - `"/absolute/path"` → absolute (internal)
pub fn is_bare_specifier(s: &str) -> bool {
    !s.starts_with('.')
        && !s.starts_with('/')
        // Windows absolute paths (e.g. "C:/...")
        && !(s.len() >= 2 && s.as_bytes()[1] == b':')
}

/// Detect references to DOM interface types declared in `lib.dom.d.ts`.
///
/// These are all global in TypeScript — no import required — and share a
/// naming convention where the prefix (`HTML`, `SVG`, `ARIA`, `CSS`, `DOM`,
/// `XML`, `RTC`, `Web`, `Animation`) signals the interface family. A pattern
/// check is used in preference to enumerating the full list because lib.dom
/// has hundreds of entries and new ones ship with each Chrome/Firefox release.
pub(crate) fn is_dom_interface_type(target: &str) -> bool {
    let first_seg = target.split('.').next().unwrap_or(target);
    if first_seg.len() < 4 {
        return false;
    }
    // Conservative: only prefixes that are unambiguously DOM interface
    // namespaces. Skipped `Web` / `Media` / `Audio` because user code commonly
    // uses names like `WebhookPayload`, `MediaItem`, `AudioSource` that would
    // false-positive.
    const PREFIXES: &[&str] = &[
        "HTML", "SVG", "ARIA", "IDB", "XPath", "MathML",
    ];
    for prefix in PREFIXES {
        if first_seg.starts_with(prefix) {
            let tail = &first_seg[prefix.len()..];
            if let Some(first_char) = tail.chars().next() {
                if first_char.is_ascii_uppercase() {
                    return true;
                }
            }
        }
    }
    false
}

/// Detect references under the globally-available `React` namespace — all
/// types in the React types package are accessible as `React.X` in files
/// that use the `jsx: react-jsx` runtime, without an explicit import.
pub(crate) fn is_react_namespace_type(target: &str) -> bool {
    target.starts_with("React.")
}

/// Detect references to browser/JS runtime globals.
///
/// Matches the object prefix for dotted names like `document.querySelector`,
/// `JSON.stringify`, `console.error`, `Promise.all`, etc.
/// Also matches standalone globals like `setTimeout`, `encodeURIComponent`.
pub(crate) fn is_js_runtime_global(target: &str) -> bool {
    if is_dom_interface_type(target) || is_react_namespace_type(target) {
        return true;
    }
    // Extract the object (first segment) for dotted names.
    let obj = target.split('.').next().unwrap_or(target);
    matches!(
        obj,
        // DOM / Browser APIs
        "document" | "window" | "navigator" | "location" | "history"
            | "localStorage" | "sessionStorage" | "performance"
            | "screen" | "visualViewport" | "matchMedia"
            // jQuery / Zepto — global in pages that inline-load the library
            | "$" | "$$" | "jQuery"
            // Global objects
            | "console" | "JSON" | "Math" | "Object" | "Array"
            | "Promise" | "RegExp" | "Date" | "Map" | "Set"
            | "WeakMap" | "WeakSet" | "Symbol" | "Proxy" | "Reflect"
            | "Error" | "TypeError" | "RangeError" | "SyntaxError"
            | "ReferenceError" | "EvalError" | "URIError"
            | "Intl" | "Number" | "String" | "Boolean" | "BigInt"
            // Node.js globals
            | "Buffer" | "process" | "global" | "globalThis" | "__dirname" | "__filename"
            | "require" | "module" | "exports"
            // Global constructors
            | "URL" | "URLSearchParams" | "Headers" | "Request" | "Response"
            | "FormData" | "Blob" | "File" | "AbortController"
            | "TextEncoder" | "TextDecoder" | "ReadableStream" | "WritableStream"
            | "WebSocket" | "EventSource" | "Worker" | "SharedWorker"
            | "MessageChannel" | "BroadcastChannel"
            | "IntersectionObserver" | "MutationObserver" | "ResizeObserver"
            | "Crypto" | "crypto"
            // Global functions
            | "setTimeout" | "setInterval" | "clearTimeout" | "clearInterval"
            | "requestAnimationFrame" | "cancelAnimationFrame"
            | "requestIdleCallback" | "cancelIdleCallback"
            | "fetch" | "atob" | "btoa"
            | "encodeURIComponent" | "decodeURIComponent"
            | "encodeURI" | "decodeURI"
            | "parseInt" | "parseFloat" | "isNaN" | "isFinite"
            | "structuredClone" | "queueMicrotask"
            | "alert" | "confirm" | "prompt"
    )
}

/// Common built-in method names that appear on Array, String, Promise, and
/// Object instances. Used as a last resort in `infer_external_namespace` when
/// no import or chain information narrows the origin.
///
/// These are only checked after all import-based checks have already failed, so
/// false positives (e.g., a project method named `map`) are suppressed: if the
/// name was resolvable via imports it would have returned earlier.
pub(super) fn is_common_builtin_method(name: &str) -> bool {
    // Strip `this.` if somehow present.
    let name = name.strip_prefix("this.").unwrap_or(name);
    matches!(
        name,
        // Array methods
        "map"
            | "filter"
            | "reduce"
            | "forEach"
            | "find"
            | "findIndex"
            | "some"
            | "every"
            | "includes"
            | "push"
            | "pop"
            | "shift"
            | "unshift"
            | "slice"
            | "splice"
            | "concat"
            | "join"
            | "sort"
            | "reverse"
            | "flat"
            | "flatMap"
            | "fill"
            | "indexOf"
            | "lastIndexOf"
            | "keys"
            | "values"
            | "entries"
            | "at"
            // String methods
            | "split"
            | "replace"
            | "replaceAll"
            | "trim"
            | "trimStart"
            | "trimEnd"
            | "toLowerCase"
            | "toUpperCase"
            | "startsWith"
            | "endsWith"
            | "match"
            | "search"
            | "substring"
            | "charAt"
            | "charCodeAt"
            | "padStart"
            | "padEnd"
            | "repeat"
            // Promise methods
            | "then"
            | "catch"
            | "finally"
            // Date methods
            | "toISOString"
            | "toLocaleDateString"
            | "toLocaleTimeString"
            | "toLocaleString"
            | "getTime"
            | "getDate"
            | "getFullYear"
            | "getMonth"
            | "getHours"
            | "getMinutes"
            | "getSeconds"
            // Object methods
            | "toString"
            | "valueOf"
            | "hasOwnProperty"
            | "toFixed"
            | "toPrecision"
            | "toExponential"
            // Iteration / async
            | "next"
            | "return"
            | "throw"
            | "Symbol.iterator"
            | "Symbol.asyncIterator"
            // DOM event / traversal methods — appear as bare calls on element /
            // NodeList / HTMLCollection locals in <script> blocks (HTML files,
            // ERB/EEX templates). No import required; methods on objects returned
            // by document.* queries.
            | "addEventListener"
            | "removeEventListener"
            | "dispatchEvent"
            | "preventDefault"
            | "stopPropagation"
            | "stopImmediatePropagation"
            | "getElementById"
            | "getElementsByClassName"
            | "getElementsByTagName"
            | "getElementsByName"
            | "querySelector"
            | "querySelectorAll"
            | "closest"
            | "matches"
            | "getAttribute"
            | "setAttribute"
            | "removeAttribute"
            | "hasAttribute"
            | "toggleAttribute"
            | "appendChild"
            | "removeChild"
            | "insertBefore"
            | "replaceChild"
            | "insertAdjacentHTML"
            | "insertAdjacentElement"
            | "insertAdjacentText"
            | "cloneNode"
            | "contains"
            | "hasChildNodes"
            | "normalize"
            | "getBoundingClientRect"
            | "getClientRects"
            | "scrollIntoView"
            | "scrollTo"
            | "scrollBy"
            | "focus"
            | "blur"
            | "click"
            | "checkValidity"
            | "reportValidity"
            | "setCustomValidity"
            // NodeList / HTMLCollection index method (e.g. `rows.item(i)`)
            | "item"
            // jQuery / Zepto DOM manipulation — extracted as bare method names
            // when $(…).addClass(…) chains are parsed from ERB/EEX templates.
            | "addClass"
            | "removeClass"
            | "toggleClass"
            | "hasClass"
            | "css"
            | "html"
            | "text"
            | "val"
            | "attr"
            | "prop"
            | "data"
            | "hide"
            | "show"
            | "toggle"
            | "fadeIn"
            | "fadeOut"
            | "fadeToggle"
            | "slideDown"
            | "slideUp"
            | "slideToggle"
            | "animate"
            | "on"
            | "off"
            | "trigger"
            | "triggerHandler"
            | "bind"
            | "unbind"
            | "one"
            | "hover"
            | "ready"
            | "appendTo"
            | "prependTo"
            | "insertAfter"
            | "after"
            | "before"
            | "prepend"
            | "append"
            | "detach"
            | "empty"
            | "replaceWith"
            | "replaceAll"
            | "wrap"
            | "unwrap"
            | "wrapAll"
            | "wrapInner"
            | "parent"
            | "parents"
            | "parentsUntil"
            | "children"
            | "siblings"
            | "nextAll"
            | "nextUntil"
            | "prev"
            | "prevAll"
            | "prevUntil"
            | "not"
            | "is"
            | "has"
            | "eq"
            | "first"
            | "last"
            | "each"
            | "index"
            | "length"
            | "get"
            | "toArray"
            | "add"
            | "addBack"
            | "end"
            | "width"
            | "height"
            | "innerWidth"
            | "innerHeight"
            | "outerWidth"
            | "outerHeight"
            | "offset"
            | "position"
            | "scrollTop"
            | "scrollLeft"
            | "serialize"
            | "serializeArray"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
///
/// TypeScript is structurally typed and more permissive than C# — we allow
/// more combinations here.
pub(crate) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property" | "class" | "variable"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "interface" | "type_alias"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class"
                | "interface"
                | "enum"
                | "type_alias"
                | "function"
                | "variable"
                | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}
