// =============================================================================
// kotlin/predicates.rs — edge-kind compatibility + namespace classification
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
        EdgeKind::Implements => matches!(sym_kind, "class" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class"),
        _ => true,
    }
}

/// Always-external Kotlin/JVM namespace roots.
const ALWAYS_EXTERNAL: &[&str] = &[
    "kotlin",
    "kotlinx",
    "java",
    "javax",
    "jakarta",
    "android",
    "androidx",
    "org.junit",
    "org.assertj",
    "io.mockk",
    "org.springframework",
    "com.fasterxml",
    "io.ktor",
];

/// Check whether a Kotlin namespace or import path is external.
pub(super) fn is_external_kotlin_namespace(
    ns: &str,
    project_ctx: Option<&ProjectContext>,
) -> bool {
    for prefix in ALWAYS_EXTERNAL {
        if ns == *prefix || ns.starts_with(&format!("{prefix}.")) {
            return true;
        }
    }

    if let Some(ctx) = project_ctx {
        return is_manifest_jvm_external(ctx, ns);
    }

    false
}

/// Check whether a Kotlin/JVM namespace is external using Maven/Gradle manifests directly.
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
    is_external_kotlin_namespace(target, project_ctx)
}
