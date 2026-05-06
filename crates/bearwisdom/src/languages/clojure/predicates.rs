// =============================================================================
// clojure/predicates.rs — Clojure builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// True for Java interop method-call forms (start with `.`, e.g. `.getBytes`)
/// and Java constructor-call forms (end with `.`, e.g. `File.`).
///
/// These can never resolve to a Clojure project symbol, so the resolver skips
/// the symbol-index lookup and immediately classifies them as external.
pub(super) fn is_java_interop(name: &str) -> bool {
    // Method call: (.getBytes s), (.close stream)
    // Constructor:  (File. path), (ByteArrayOutputStream.)
    (name.starts_with('.') && name.len() > 1) || (name.ends_with('.') && name.len() > 1)
}

/// True for fully-qualified Java class references that contain internal dots,
/// e.g. `java.io.ByteArrayOutputStream.`, `java.lang.Thread`, `javax.servlet.http.HttpServletRequest`.
///
/// Clojure namespace names also contain dots (e.g. `ring.util.codec`) but those
/// are already handled by the import alias lookup path. This guard fires only
/// for names that look like Java package paths because they start with a
/// well-known Java top-level package (`java.`, `javax.`, `org.`, `com.`, `sun.`,
/// `io.`, `net.`) which Clojure namespaces effectively never use as a prefix.
pub(super) fn is_java_class_ref(name: &str) -> bool {
    if !name.contains('.') {
        return false;
    }
    // Names ending with `.` that also have internal `.` separators are
    // fully-qualified constructor calls (already caught by is_java_interop,
    // but guard here as well for clarity).
    let check = name.trim_end_matches('.');
    matches!(
        check.split('.').next().unwrap_or(""),
        "java"
            | "javax"
            | "org"
            | "com"
            | "sun"
            | "io"
            | "net"
            | "edu"
            | "gov"
            | "mil"
    )
}

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(sym_kind, "method" | "function" | "constructor" | "test" | "class"),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "function" | "variable"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "function"),
        _ => true,
    }
}
