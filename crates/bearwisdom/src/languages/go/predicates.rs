// =============================================================================
// go/predicates.rs — Go builtin and helper predicates
// =============================================================================

use crate::indexer::resolve::engine::SymbolInfo;
use crate::types::EdgeKind;

/// Extract the Go package name from a symbol's qualified_name.
/// "main.Server" → "main", "handlers.Handler" → "handlers".
pub(super) fn sym_package(sym: &SymbolInfo) -> &str {
    sym.qualified_name
        .split('.')
        .next()
        .unwrap_or(sym.qualified_name.as_str())
}

/// Return the directory portion of a file path (everything up to the last '/').
pub(super) fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[..pos],
        None => "",
    }
}

/// Check whether a Go import path is external (heuristic: host with a dot).
/// Used when no ProjectContext is available.
pub(super) fn is_external_go_import_fallback(import_path: &str) -> bool {
    let first_segment = import_path.split('/').next().unwrap_or(import_path);
    first_segment.contains('.')
}

/// Go built-in functions and types that are always in scope without import.
/// These come from the `builtin` pseudo-package.
pub(super) fn is_go_builtin(name: &str) -> bool {
    matches!(
        name,
        "len" | "cap" | "make" | "new" | "append" | "copy" | "delete"
            | "close" | "panic" | "recover" | "print" | "println"
            | "complex" | "real" | "imag" | "clear" | "min" | "max"
            // Built-in type conversions used as calls
            | "string" | "int" | "int8" | "int16" | "int32" | "int64"
            | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "float32" | "float64" | "byte" | "rune" | "bool"
            | "error" | "any" | "comparable"
    )
}

/// Detect Go composite literal types that the extractor captures as target_name.
/// Examples: `[]string`, `map[string]int`, `[]*Foo`, `[]tests.ApiScenario`.
pub(super) fn is_go_composite_type(name: &str) -> bool {
    name.starts_with("[]")
        || name.starts_with("[]*")
        || name.starts_with("map[")
        || name.starts_with("chan ")
        || name.starts_with("*[")
}

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "struct" | "interface"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "struct" | "class"),
        _ => true,
    }
}
