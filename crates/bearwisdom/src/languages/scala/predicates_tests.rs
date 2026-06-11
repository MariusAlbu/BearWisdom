// Tests for Scala/JVM external-namespace classification after the
// third-party-group-id purge. Platform roots (scala/java/javax/jakarta) stay;
// third-party group-ids are classified from the Maven/Gradle manifest. With no
// manifest, a third-party namespace is unresolved (sbt projects keep the
// multi-segment fallback, exercised separately).

use super::predicates::{is_external_scala_namespace, is_manifest_jvm_external};
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
        &["com.typesafe.akka", "org.typelevel", "dev.zio"],
    );
    assert!(is_manifest_jvm_external(&ctx, "com.typesafe.akka.actor.Actor"));
    assert!(is_manifest_jvm_external(&ctx, "org.typelevel.cats.effect.IO"));
    assert!(is_manifest_jvm_external(&ctx, "dev.zio.ZIO"));

    // Gradle path is equivalent.
    let g = ctx_with(ManifestKind::Gradle, &["org.http4s"]);
    assert!(is_manifest_jvm_external(&g, "org.http4s.dsl.Http4sDsl"));
}

#[test]
fn group_id_without_manifest_is_not_external() {
    // No Maven/Gradle manifest → third-party group-ids no longer short-circuit.
    assert!(!is_external_scala_namespace("akka.actor.Actor", None));
    assert!(!is_external_scala_namespace("cats.effect.IO", None));
    assert!(!is_external_scala_namespace("zio.ZIO", None));
    assert!(!is_external_scala_namespace("doobie.Transactor", None));

    // Empty Gradle manifest present but the group-id isn't declared → not external.
    let ctx = ctx_with(ManifestKind::Gradle, &[]);
    assert!(!is_external_scala_namespace("akka.actor.Actor", Some(&ctx)));
}

#[test]
fn platform_roots_still_classify() {
    // Scala + JVM platform substrate — no manifest needed.
    assert!(is_external_scala_namespace("scala.collection.immutable.List", None));
    assert!(is_external_scala_namespace("java.util.List", None));
    assert!(is_external_scala_namespace("javax.inject.Inject", None));
    assert!(is_external_scala_namespace("jakarta.persistence.Entity", None));
}

#[test]
fn arbitrary_org_root_is_not_external_without_manifest() {
    // The previous overbroad `root == "org"` clause is gone: a bare `org.*`
    // namespace must NOT classify external on the group-id name alone.
    assert!(!is_manifest_jvm_external(
        &ProjectContext::default(),
        "org.scalatest.flatspec.AnyFlatSpec"
    ));
    assert!(!is_manifest_jvm_external(
        &ProjectContext::default(),
        "org.specs2.Specification"
    ));
}
