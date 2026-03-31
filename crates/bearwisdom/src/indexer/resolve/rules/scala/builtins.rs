// =============================================================================
// scala/builtins.rs — Scala builtin and helper predicates
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(super) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "trait"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "trait" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "trait" | "interface" | "enum" | "type_alias" | "namespace" | "object"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Always-external Scala/JVM namespace roots.
const ALWAYS_EXTERNAL: &[&str] = &[
    "scala",
    "java",
    "javax",
    "jakarta",
    "akka",
    "cats",
    "zio",
    "fs2",
    "http4s",
    "io.circe",
    "circe",
    "play",
    "org.scalatest",
    "org.specs2",
    "org.scalamock",
    "com.typesafe",
    "slick",
    "doobie",
    "pekko",
];

/// Check whether a Scala namespace or import path is external.
pub(super) fn is_external_scala_namespace(
    ns: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    if let Some(ctx) = project_ctx {
        return ctx.is_external_namespace(ns);
    }

    false
}

/// Check whether a fully-qualified target looks external.
pub(super) fn effective_target_is_external(
    target: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    if !target.contains('.') {
        return false;
    }
    is_external_scala_namespace(target, project_ctx)
}

/// Scala stdlib builtins always in scope via `scala.Predef` and `scala.*`.
pub(super) fn is_scala_builtin(name: &str) -> bool {
    let root = name.split('.').next().unwrap_or(name);
    matches!(
        root,
        // Predef functions / implicit conversions
        "println"
            | "print"
            | "printf"
            | "require"
            | "assert"
            | "assume"
            | "identity"
            | "implicitly"
            | "locally"
            | "summon"
            // Scala 3
            | "using"
            // Placeholder for unimplemented
            | "???"
            // Core types (scala.*)
            | "String"
            | "Int"
            | "Long"
            | "Double"
            | "Float"
            | "Boolean"
            | "Byte"
            | "Short"
            | "Char"
            | "Unit"
            | "Nothing"
            | "Any"
            | "AnyVal"
            | "AnyRef"
            | "Null"
            | "Option"
            | "Some"
            | "None"
            | "Either"
            | "Left"
            | "Right"
            | "List"
            | "Nil"
            | "Map"
            | "Set"
            | "Vector"
            | "Seq"
            | "IndexedSeq"
            | "Array"
            | "Range"
            | "Tuple2"
            | "Tuple3"
            | "Future"
            | "Promise"
            | "Try"
            | "Success"
            | "Failure"
            | "Iterator"
            | "Iterable"
            | "Traversable"
            // Companion object methods often used bare
            | "apply"
            | "unapply"
            | "unapplySeq"
            | "empty"
            | "newBuilder"
            // pseudo-keywords used as refs
            | "this"
            | "super"
    )
}
