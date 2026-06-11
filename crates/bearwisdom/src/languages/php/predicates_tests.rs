// Tests for PHP external-namespace classification after the framework-list
// purge. Composer is the only path: a namespace is external iff `composer.json`
// declares the owning package. With no manifest, the namespace is unresolved.

use super::predicates::{is_external_php_namespace, is_manifest_php_external};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;

fn ctx_with_composer(deps: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut composer = ManifestData::default();
    for d in deps {
        composer.dependencies.insert((*d).to_string());
    }
    ctx.manifests.insert(ManifestKind::Composer, composer);
    ctx
}

#[test]
fn composer_declared_package_classifies_external() {
    // Composer names are `vendor/package`; the namespace root matches the
    // package segment (`monolog/monolog` → root segment "monolog").
    let ctx = ctx_with_composer(&["monolog/monolog", "ramsey/uuid"]);
    assert!(is_manifest_php_external(&ctx, "monolog"));
    assert!(is_manifest_php_external(&ctx, "monolog.Logger"));
    assert!(is_manifest_php_external(&ctx, "uuid.Uuid"));
}

#[test]
fn namespace_without_composer_is_not_external() {
    // No ProjectContext → no manifest → not external (honest-unresolved). The
    // formerly hardcoded framework roots no longer short-circuit.
    assert!(!is_external_php_namespace("Illuminate.Support.Collection", None));
    assert!(!is_external_php_namespace("Symfony.Component.Console", None));
    assert!(!is_external_php_namespace("Doctrine.ORM.EntityManager", None));
    assert!(!is_external_php_namespace("PHPUnit.Framework.TestCase", None));

    // Empty composer.json present but the package isn't declared → not external.
    let ctx = ctx_with_composer(&[]);
    assert!(!is_external_php_namespace("Illuminate.Support", Some(&ctx)));
}

#[test]
fn composer_path_drives_classification() {
    // With the package declared, the same namespace classifies external.
    let ctx = ctx_with_composer(&["laravel/framework"]);
    assert!(is_external_php_namespace("framework.Support", Some(&ctx)));
}
