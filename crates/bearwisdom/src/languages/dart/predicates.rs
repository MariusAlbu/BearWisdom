// =============================================================================
// dart/predicates.rs — Dart builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Check whether a Dart import URI is external (stdlib or pub package).
pub(super) fn is_external_dart_import(uri: &str) -> bool {
    // dart: scheme = stdlib
    if uri.starts_with("dart:") {
        return true;
    }
    // package: scheme = pub dependency
    if uri.starts_with("package:") {
        return true;
    }
    false
}

/// Check whether a Dart import URI is project-local (relative or unqualified).
#[allow(dead_code)]
pub(super) fn is_relative_dart_import(uri: &str) -> bool {
    uri.starts_with('.') || (!uri.starts_with("dart:") && !uri.starts_with("package:"))
}

/// Dart primitive type names + universal language tokens that the
/// extractor emits as type_identifier nodes. Filtered at extract time.
/// Stdlib types (String, List, Map, Future, Stream, ...) flow through
/// and resolve via the dart_sdk walker.
pub(super) fn is_dart_primitive_type(name: &str) -> bool {
    matches!(
        name,
        // Numeric / boolean primitives
        "int" | "double" | "num" | "bool"
        // Empty / dynamic / never types
        | "void" | "dynamic" | "Never" | "Null"
        // Universal literals
        | "true" | "false" | "null"
    )
}
