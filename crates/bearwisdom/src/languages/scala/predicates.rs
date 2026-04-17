// =============================================================================
// scala/predicates.rs — Scala builtin and helper predicates
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
        // If we have a JVM manifest, use it for precise classification.
        let has_jvm_manifest = ctx.manifests.contains_key(&ManifestKind::Maven)
            || ctx.manifests.contains_key(&ManifestKind::Gradle);
        if has_jvm_manifest {
            return is_manifest_jvm_external(ctx, ns);
        }
        // No JVM manifest (sbt projects without build.gradle/pom.xml):
        // treat multi-segment namespaces as external since we can't
        // distinguish project-local from third-party without a manifest.
        // Single-segment or project-package-prefixed names fall through.
        if ns.contains('.') {
            return true;
        }
    }

    false
}

/// Check whether a Scala/JVM namespace is external using Maven/Gradle manifests directly.
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
            | "copy"
            | "empty"
            | "newBuilder"
            // pseudo-keywords used as refs
            | "this"
            | "super"
            // Object identity / equality (java.lang.Object methods always in scope)
            | "toString"
            | "hashCode"
            | "equals"
            | "canEqual"
            // Cats / FP symbolic operators (method dispatch, not imported names)
            | "*>"
            | "<*"
            | "==="
            | "=!="
            | ">>"
            | ">>="
            | "<*>"
            | "<$>"
            | "|+|"
            | ">>>"
            | "<<<"
            | "&>"
            | "<&"
            // Universal FP method names (stdlib + Cats + any typeclass)
            | "flatMap"
            | "map"
            | "fold"
            | "foldLeft"
            | "foldRight"
            | "traverse"
            | "sequence"
            | "pure"
            | "flatten"
            | "filter"
            | "collect"
            | "exists"
            | "forall"
            | "foreach"
            | "groupBy"
            | "toList"
            | "toVector"
            | "toSet"
            | "toMap"
            | "toOption"
            | "getOrElse"
            | "orElse"
            | "contains"
            | "mkString"
            | "zip"
            | "zipWithIndex"
            | "take"
            | "drop"
            | "head"
            | "tail"
            | "last"
            | "headOption"
            | "lastOption"
            | "isEmpty"
            | "nonEmpty"
            | "size"
            | "length"
            // Effect / stream methods (fs2, cats-effect, http4s)
            | "unsafeRunSync"
            | "use"
            | "evalMap"
            | "compile"
            | "drain"
            | "through"
            | "attempt"
            | "handleErrorWith"
            | "recoverWith"
            | "void"
            | "as"
            | "tupleLeft"
            | "tupleRight"
            | "product"
            | "productL"
            | "productR"
            // ScalaCheck / property-based testing
            | "Gen"
            | "Arbitrary"
            | "forAll"
            | "property"
    )
}
