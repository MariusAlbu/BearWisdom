// =============================================================================
// dart/builtins.rs — Dart builtin and helper predicates
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

/// Always-external Dart package prefixes (retained for future use).
#[allow(dead_code)]
const ALWAYS_EXTERNAL: &[&str] = &[
    "dart:",
    "package:flutter",
    "package:provider",
    "package:riverpod",
    "package:flutter_riverpod",
    "package:bloc",
    "package:flutter_bloc",
    "package:dio",
    "package:http",
    "package:get",
    "package:get_it",
    "package:injectable",
    "package:freezed",
    "package:json_annotation",
    "package:hive",
    "package:isar",
    "package:sqflite",
    "package:firebase_core",
    "package:firebase_auth",
    "package:cloud_firestore",
    "package:go_router",
    "package:auto_route",
    "package:mockito",
    "package:flutter_test",
    "package:test",
];

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

/// Dart builtin types and functions always in scope.
pub(super) fn is_dart_builtin(name: &str) -> bool {
    matches!(
        name,
        // Core functions
        "print"
            | "debugPrint"
            | "identical"
            | "identityHashCode"
            // setState is Widget-level but universally used
            | "setState"
            | "mounted"
            // Core types (dart:core — always in scope)
            | "String"
            | "int"
            | "double"
            | "bool"
            | "num"
            | "List"
            | "Map"
            | "Set"
            | "Iterable"
            | "Iterator"
            | "Object"
            | "dynamic"
            | "void"
            | "Never"
            | "Null"
            | "Function"
            | "Type"
            | "Symbol"
            | "Future"
            | "Stream"
            | "Completer"
            | "Duration"
            | "DateTime"
            | "Uri"
            | "RegExp"
            | "Pattern"
            | "Match"
            | "Error"
            | "Exception"
            | "AssertionError"
            | "RangeError"
            | "StateError"
            | "ArgumentError"
            | "TypeError"
            | "UnsupportedError"
            | "UnimplementedError"
            | "StackOverflowError"
            | "OutOfMemoryError"
            | "FormatException"
            | "NullThrownError"
            | "IntegerDivisionByZeroException"
            | "Comparable"
            | "Enum"
            | "Record"
            | "Sink"
            | "StreamController"
            | "StreamSubscription"
            | "StringBuffer"
            | "StringSink"
            | "Stopwatch"
            | "RuneIterator"
            | "BigInt"
            // Common annotations (dart:core)
            | "override"
            | "deprecated"
            | "Deprecated"
            | "required"
            | "visibleForTesting"
            | "immutable"
            | "sealed"
            | "pragma"
    )
}
