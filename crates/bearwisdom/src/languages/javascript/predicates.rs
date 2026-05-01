// =============================================================================
// javascript/predicates.rs — JavaScript builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
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
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}

/// Check whether a name is a JavaScript/Node.js runtime global or well-known
/// stdlib identifier that will never appear in the project symbol index.
pub(super) fn is_javascript_builtin(name: &str) -> bool {
    // Strip object prefix for dotted names (`process.env` → check `process`).
    let root = name.split('.').next().unwrap_or(name);

    matches!(
        root,
        // ── Node.js globals ──────────────────────────────────────────────────
        "require"
            | "module"
            | "exports"
            | "__dirname"
            | "__filename"
            | "process"
            | "global"
            | "globalThis"
            // ── I/O & encoding ───────────────────────────────────────────────
            | "Buffer"
            | "console"
            | "TextEncoder"
            | "TextDecoder"
            | "atob"
            | "btoa"
            | "structuredClone"
            | "queueMicrotask"
            // ── Timers ───────────────────────────────────────────────────────
            | "setTimeout"
            | "setInterval"
            | "clearTimeout"
            | "clearInterval"
            | "setImmediate"
            | "clearImmediate"
            // ── Networking / fetch ───────────────────────────────────────────
            | "fetch"
            | "URL"
            | "URLSearchParams"
            | "Headers"
            | "Request"
            | "Response"
            | "AbortController"
            | "AbortSignal"
            | "FormData"
            | "Blob"
            | "File"
            | "ReadableStream"
            | "WritableStream"
            | "WebSocket"
            // ── Crypto / performance ─────────────────────────────────────────
            | "crypto"
            | "Crypto"
            | "SubtleCrypto"
            | "performance"
            // ── ECMAScript built-in objects ───────────────────────────────────
            | "Object"
            | "Array"
            | "Function"
            | "String"
            | "Number"
            | "Boolean"
            | "Symbol"
            | "BigInt"
            | "Math"
            | "Date"
            | "RegExp"
            | "JSON"
            | "Map"
            | "Set"
            | "WeakMap"
            | "WeakSet"
            | "WeakRef"
            | "FinalizationRegistry"
            | "Promise"
            | "Proxy"
            | "Reflect"
            | "Error"
            | "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "EvalError"
            | "URIError"
            | "AggregateError"
            | "Intl"
            | "ArrayBuffer"
            | "SharedArrayBuffer"
            | "DataView"
            | "Int8Array"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Int16Array"
            | "Uint16Array"
            | "Int32Array"
            | "Uint32Array"
            | "Float32Array"
            | "Float64Array"
            | "BigInt64Array"
            | "BigUint64Array"
            | "Iterator"
            | "Generator"
            | "GeneratorFunction"
            | "AsyncFunction"
            | "AsyncGenerator"
            | "AsyncGeneratorFunction"
            // ── ECMAScript global functions ───────────────────────────────────
            | "parseInt"
            | "parseFloat"
            | "isNaN"
            | "isFinite"
            | "encodeURIComponent"
            | "decodeURIComponent"
            | "encodeURI"
            | "decodeURI"
            | "eval"
            | "undefined"
            | "NaN"
            | "Infinity"
            // ── Browser / DOM globals (often appear in JS files) ─────────────
            | "window"
            | "document"
            | "navigator"
            | "location"
            | "history"
            | "localStorage"
            | "sessionStorage"
            | "screen"
            | "alert"
            | "confirm"
            | "prompt"
            | "requestAnimationFrame"
            | "cancelAnimationFrame"
            | "requestIdleCallback"
            | "cancelIdleCallback"
            | "XMLHttpRequest"
            | "EventSource"
            | "Worker"
            | "SharedWorker"
            | "MessageChannel"
            | "BroadcastChannel"
            | "IntersectionObserver"
            | "MutationObserver"
            | "ResizeObserver"
            // ── Node.js core module names (bare specifiers) ──────────────────
            | "fs"
            | "path"
            | "http"
            | "https"
            | "net"
            | "os"
            | "child_process"
            | "cluster"
            | "dgram"
            | "dns"
            | "events"
            | "readline"
            | "stream"
            | "url"
            | "util"
            | "zlib"
            | "buffer"
            | "querystring"
            | "string_decoder"
            | "tls"
            | "tty"
            | "v8"
            | "vm"
            | "worker_threads"
            | "perf_hooks"
            | "async_hooks"
            | "repl"
            | "inspector"
            | "trace_events"
    )
}
