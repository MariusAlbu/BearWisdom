// Tests for Kotlin/JVM external-namespace classification after the
// third-party-group-id purge. Platform roots (java/javax/jakarta/kotlin/
// android/…) stay; third-party group-ids are classified from the Maven/Gradle
// manifest. With no manifest, a third-party namespace is unresolved.

use super::predicates::{is_external_kotlin_namespace, is_manifest_jvm_external};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;

fn ctx_with(kind: ManifestKind, group_ids: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut m = ManifestData::default();
    for g in group_ids {
        m.dependencies.insert((*g).to_string());
    }
    ctx.manifests.insert(kind, m);
    ctx
}

#[test]
fn manifest_declared_group_id_classifies_external() {
    let ctx = ctx_with(
        ManifestKind::Maven,
        &["org.springframework", "io.ktor", "com.fasterxml.jackson.core"],
    );
    assert!(is_manifest_jvm_external(&ctx, "org.springframework.boot.Application"));
    assert!(is_manifest_jvm_external(&ctx, "io.ktor.server.engine"));
    assert!(is_manifest_jvm_external(&ctx, "com.fasterxml.jackson.core.JsonParser"));

    // Gradle path is equivalent.
    let g = ctx_with(ManifestKind::Gradle, &["org.junit.jupiter"]);
    assert!(is_manifest_jvm_external(&g, "org.junit.jupiter.api.Test"));
}

#[test]
fn group_id_without_manifest_is_not_external() {
    // No Maven/Gradle manifest → third-party group-ids no longer short-circuit.
    assert!(!is_external_kotlin_namespace("org.springframework.web", None));
    assert!(!is_external_kotlin_namespace("io.ktor.client", None));
    assert!(!is_external_kotlin_namespace("com.fasterxml.jackson", None));
    assert!(!is_external_kotlin_namespace("org.assertj.core", None));

    // Empty manifest present but the group-id isn't declared → not external.
    let ctx = ctx_with(ManifestKind::Gradle, &[]);
    assert!(!is_external_kotlin_namespace("org.springframework.web", Some(&ctx)));
}

#[test]
fn platform_roots_still_classify() {
    // JVM + Kotlin + Android platform substrate — no manifest needed.
    assert!(is_external_kotlin_namespace("java.util.List", None));
    assert!(is_external_kotlin_namespace("javax.inject.Inject", None));
    assert!(is_external_kotlin_namespace("jakarta.persistence.Entity", None));
    assert!(is_external_kotlin_namespace("kotlin.collections.List", None));
    assert!(is_external_kotlin_namespace("kotlinx.coroutines.flow", None));
    assert!(is_external_kotlin_namespace("android.os.Bundle", None));
    assert!(is_external_kotlin_namespace("androidx.compose.runtime", None));
}

#[test]
fn arbitrary_org_root_is_not_external_without_manifest() {
    // The previous overbroad `root == "org"` clause is gone: a bare `org.*`
    // namespace must NOT classify external on the group-id name alone.
    assert!(!is_external_kotlin_namespace("org.example.app.Main", None));
    assert!(!is_manifest_jvm_external(
        &ProjectContext::default(),
        "org.springframework.web"
    ));
}
