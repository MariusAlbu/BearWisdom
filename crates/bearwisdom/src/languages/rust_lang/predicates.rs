// =============================================================================
// rust_lang/predicates.rs — Rust builtin and helper predicates
// =============================================================================

use crate::types::{EdgeKind, ParsedFile};

use crate::indexer::resolve::engine::SymbolInfo;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "struct" | "interface" | "trait"),
        EdgeKind::Implements => matches!(sym_kind, "interface" | "trait"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class"
                | "struct"
                | "interface"
                | "enum"
                | "enum_member"
                | "type_alias"
                | "trait"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "struct" | "class"),
        _ => true,
    }
}

/// Variant of `kind_compatible` that consults the candidate symbol's
/// signature to allow Calls→Variable when the variable is callable
/// (an `impl Fn(...)` parameter, a closure-typed binding, or a `fn(...)`
/// function pointer). Used by the resolver wherever a method/function
/// call could legitimately bind to such a variable; the plain
/// `kind_compatible` check still drives the general case.
pub(super) fn kind_compatible_with_signature(
    edge_kind: EdgeKind,
    sym: &SymbolInfo,
) -> bool {
    if kind_compatible(edge_kind, &sym.kind) {
        return true;
    }
    if edge_kind == EdgeKind::Calls && sym.kind == "variable" {
        if let Some(sig) = sym.signature.as_deref() {
            return super::symbols::is_callable_rust_type(sig);
        }
    }
    false
}

/// Extract the module path from a parsed Rust file.
/// The Rust extractor sets scope_path on top-level symbols to the module path,
/// e.g., "crate::models" or "crate::api::handlers".
pub(super) fn extract_module_path(file: &ParsedFile) -> Option<String> {
    for sym in &file.symbols {
        if let Some(ref sp) = sym.scope_path {
            if !sp.is_empty() {
                // scope_path may use `::` or `.` separators — normalize to `.`
                let dot_path = sp.replace("::", ".");
                return Some(dot_path);
            }
        }
        // If no scope_path, check qualified_name prefix.
        if let Some(dot) = sym.qualified_name.rfind('.') {
            let prefix = &sym.qualified_name[..dot];
            if !prefix.is_empty() {
                return Some(prefix.to_string());
            }
        }
    }
    None
}

/// Normalize a Rust `::` path to the `.`-separated form used in the symbol index.
/// "crate::models::User" → "crate.models.User"
/// "serde::Deserialize"  → "serde.Deserialize"
pub(super) fn normalize_path(s: &str) -> String {
    s.replace("::", ".")
}

/// Extract the module prefix from a symbol's qualified_name.
/// "crate.models.User" → "crate.models"
pub(super) fn sym_module(sym: &SymbolInfo) -> &str {
    match sym.qualified_name.rfind('.') {
        Some(pos) => &sym.qualified_name[..pos],
        None => "",
    }
}

/// Return the directory portion of a file path.
pub(super) fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[..pos],
        None => "",
    }
}
