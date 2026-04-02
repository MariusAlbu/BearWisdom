// =============================================================================
// typescript/builtins.rs — TypeScript/JavaScript builtin and helper predicates
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

/// Detect references to browser/JS runtime globals.
///
/// Matches the object prefix for dotted names like `document.querySelector`,
/// `JSON.stringify`, `console.error`, `Promise.all`, etc.
/// Also matches standalone globals like `setTimeout`, `encodeURIComponent`.
pub(super) fn is_js_runtime_global(target: &str) -> bool {
    // Extract the object (first segment) for dotted names.
    let obj = target.split('.').next().unwrap_or(target);
    matches!(
        obj,
        // DOM / Browser APIs
        "document" | "window" | "navigator" | "location" | "history"
            | "localStorage" | "sessionStorage" | "performance"
            // Global objects
            | "console" | "JSON" | "Math" | "Object" | "Array"
            | "Promise" | "RegExp" | "Date" | "Map" | "Set"
            | "WeakMap" | "WeakSet" | "Symbol" | "Proxy" | "Reflect"
            | "Error" | "TypeError" | "RangeError" | "SyntaxError"
            | "Intl" | "Number" | "String" | "Boolean"
            // Global functions
            | "setTimeout" | "setInterval" | "clearTimeout" | "clearInterval"
            | "requestAnimationFrame" | "cancelAnimationFrame"
            | "fetch" | "atob" | "btoa"
            | "encodeURIComponent" | "decodeURIComponent"
            | "encodeURI" | "decodeURI"
            | "parseInt" | "parseFloat" | "isNaN" | "isFinite"
            | "structuredClone" | "queueMicrotask"
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
            // Object methods
            | "toString"
            | "valueOf"
            | "hasOwnProperty"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
///
/// TypeScript is structurally typed and more permissive than C# — we allow
/// more combinations here.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property" | "class"
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
