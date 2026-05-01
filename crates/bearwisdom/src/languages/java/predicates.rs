// =============================================================================
// java/predicates.rs — Java builtin and helper predicates
// =============================================================================

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Return the first directory segment of a path — used as a crude package boundary.
pub(super) fn first_segment(path: &str) -> &str {
    match path.find('/') {
        Some(pos) => &path[..pos],
        None => path,
    }
}

/// Always-external Java namespace roots (stdlib + test frameworks).
const ALWAYS_EXTERNAL: &[&str] = &[
    "java",
    "javax",
    "jakarta",
    "org.junit",
    "sun",
    "com.sun",
];

/// Check whether a Java namespace or import path is external.
pub(super) fn is_external_java_namespace(
    ns: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    // Always-external first.
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    // Check manifest-based JVM dependencies (from pom.xml / build.gradle).
    if let Some(ctx) = project_ctx {
        return is_manifest_jvm_external(ctx, ns);
    }

    false
}

/// Check whether a Java namespace is external using Maven/Gradle manifests directly.
pub(super) fn is_manifest_jvm_external(ctx: &ProjectContext, ns: &str) -> bool {
    let root = ns.split('.').next().unwrap_or(ns);
    if matches!(root, "java" | "javax" | "jakarta" | "sun" | "org") {
        return true;
    }
    for kind in [ManifestKind::Maven, ManifestKind::Gradle] {
        if let Some(m) = ctx.manifest(kind) {
            if m.dependencies.contains(ns) {
                return true;
            }
            for dep in &m.dependencies {
                if ns.starts_with(dep.as_str()) {
                    return true;
                }
            }
        }
    }
    false
}

/// Check whether a target reference that is already fully-qualified looks external.
pub(super) fn effective_target_is_external(
    target: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    if !target.contains('.') {
        return false;
    }
    is_external_java_namespace(target, project_ctx)
}

/// Java built-in methods and types always in scope without import (java.lang.*
/// plus common Collection, Stream, and test framework method names).
///
/// The root check covers `System.out.println` → root = "System".
/// The method name check catches instance-method calls like `list.add(x)` where
/// the receiver type is a stdlib Collection — these will never be in the project index.
pub(super) fn is_java_builtin(name: &str) -> bool {
    // Extract the object prefix for dotted names like `System.out.println`.
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // Contextual keyword (Java 10+): `var x = ...` — the extractor emits
        // this as a type_identifier when used as a local var type inference.
        "var"
            // java.lang types always visible
            | "System"
            | "String"
            | "Integer"
            | "Long"
            | "Double"
            | "Float"
            | "Boolean"
            | "Byte"
            | "Short"
            | "Character"
            | "Object"
            | "Class"
            | "Enum"
            | "Record"
            | "Math"
            | "StrictMath"
            | "StringBuilder"
            | "StringBuffer"
            | "Thread"
            | "Runnable"
            | "Exception"
            | "RuntimeException"
            | "Error"
            // java.lang exception hierarchy (always visible, never imported)
            | "Throwable"
            | "IllegalArgumentException"
            | "IllegalStateException"
            | "NullPointerException"
            | "IndexOutOfBoundsException"
            | "ArrayIndexOutOfBoundsException"
            | "StringIndexOutOfBoundsException"
            | "NumberFormatException"
            | "ClassCastException"
            | "ClassNotFoundException"
            | "NoSuchMethodException"
            | "NoSuchFieldException"
            | "ArithmeticException"
            | "UnsupportedOperationException"
            | "InterruptedException"
            | "SecurityException"
            | "OutOfMemoryError"
            | "StackOverflowError"
            | "AssertionError"
            | "NoClassDefFoundError"
            | "Iterable"
            | "Comparable"
            | "Cloneable"
            | "AutoCloseable"
            | "Override"
            | "Deprecated"
            | "SuppressWarnings"
            // Pseudo-builtin calls
            | "super"
            | "this"
            // Object methods (always available without import)
            | "toString"
            | "equals"
            | "hashCode"
            | "getClass"
            | "notify"
            | "notifyAll"
            | "wait"
            | "clone"
            // String instance methods
            | "length"
            | "charAt"
            | "substring"
            | "indexOf"
            | "lastIndexOf"
            | "contains"
            | "startsWith"
            | "endsWith"
            | "replace"
            | "replaceAll"
            | "split"
            | "trim"
            | "toLowerCase"
            | "toUpperCase"
            | "isEmpty"
            | "toCharArray"
            | "valueOf"
            // Collection / List / Map / Set methods (java.util, always imported)
            | "add"
            | "remove"
            | "size"
            | "clear"
            | "iterator"
            | "toArray"
            | "stream"
            | "forEach"
            | "get"
            | "set"
            | "put"
            | "containsKey"
            | "containsValue"
            | "keySet"
            | "values"
            | "entrySet"
            // Stream methods (java.util.stream)
            | "map"
            | "filter"
            | "reduce"
            | "collect"
            | "findFirst"
            | "findAny"
            | "anyMatch"
            | "allMatch"
            | "noneMatch"
            | "count"
            | "sorted"
            | "distinct"
            | "limit"
            | "skip"
            | "flatMap"
            | "peek"
            | "toList"
    )
}
