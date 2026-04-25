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
