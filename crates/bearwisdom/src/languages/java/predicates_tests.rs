// Tests for Java/JVM external-namespace classification after the
// third-party-group-id purge. Platform roots (java/javax/jakarta/sun/com.sun)
// stay; third-party group-ids (JUnit, Spring, …) are classified from the
// Maven/Gradle manifest. With no manifest, a third-party namespace is
// unresolved.

use super::predicates::{is_external_java_namespace, is_manifest_jvm_external};
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
        &["org.junit.jupiter", "org.springframework", "com.fasterxml.jackson.core"],
    );
    assert!(is_manifest_jvm_external(&ctx, "org.junit.jupiter.api.Test"));
    assert!(is_manifest_jvm_external(&ctx, "org.springframework.boot.Application"));
    assert!(is_manifest_jvm_external(&ctx, "com.fasterxml.jackson.core.JsonParser"));

    // Gradle path is equivalent.
    let g = ctx_with(ManifestKind::Gradle, &["org.assertj"]);
    assert!(is_manifest_jvm_external(&g, "org.assertj.core.api.Assertions"));
}

#[test]
fn junit_without_manifest_is_not_external() {
    // `org.junit` was a third-party entry in the purged list. With no manifest
    // declaring it, a JUnit namespace no longer short-circuits to external.
    assert!(!is_external_java_namespace("org.junit.jupiter.api.Test", None));
    assert!(!is_external_java_namespace("org.junit.Assert", None));

    // Empty manifest present but the group-id isn't declared → not external.
    let ctx = ctx_with(ManifestKind::Gradle, &[]);
    assert!(!is_external_java_namespace("org.junit.jupiter.api.Test", Some(&ctx)));
}

#[test]
fn platform_roots_still_classify() {
    // JDK platform substrate — no manifest needed.
    assert!(is_external_java_namespace("java.util.List", None));
    assert!(is_external_java_namespace("javax.inject.Inject", None));
    assert!(is_external_java_namespace("jakarta.persistence.Entity", None));
    assert!(is_external_java_namespace("sun.misc.Unsafe", None));
    assert!(is_external_java_namespace("com.sun.net.httpserver.HttpServer", None));
}

#[test]
fn arbitrary_org_root_is_not_external_without_manifest() {
    // The previous overbroad `root == "org"` clause is gone: a bare `org.*`
    // namespace must NOT classify external on the group-id name alone.
    assert!(!is_manifest_jvm_external(
        &ProjectContext::default(),
        "org.springframework.web"
    ));
    assert!(!is_manifest_jvm_external(
        &ProjectContext::default(),
        "org.junit.jupiter.api.Test"
    ));
}
