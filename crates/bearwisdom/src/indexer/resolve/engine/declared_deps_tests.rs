use std::collections::HashMap;

use super::DeclaredDeps;
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;

fn manifest_with(deps: &[&str]) -> ManifestData {
    let mut m = ManifestData::default();
    for d in deps {
        m.dependencies.insert((*d).to_string());
    }
    m
}

fn ctx_union(deps: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    ctx.manifests.insert(ManifestKind::Npm, manifest_with(deps));
    ctx
}

#[test]
fn union_probe_matches_exact_and_deep_specifiers() {
    let deps = DeclaredDeps::snapshot(&ctx_union(&["lodash", "@scope/pkg"]));
    assert!(deps.contains(None, "lodash"));
    assert!(deps.contains(None, "lodash/fp"));
    assert!(deps.contains(None, "@scope/pkg"));
    assert!(deps.contains(None, "@scope/pkg/sub/mod"));
    assert!(!deps.contains(None, "leftpad"));
    assert!(!deps.contains(None, "./relative"));
}

#[test]
fn underscore_import_matches_hyphenated_manifest_name() {
    let deps = DeclaredDeps::snapshot(&ctx_union(&["flask-babel"]));
    assert!(deps.contains(None, "flask_babel"));
}

#[test]
fn isolated_package_never_borrows_a_sibling_dependency() {
    let mut ctx = ctx_union(&["lodash", "playwright"]);
    let mut web: HashMap<ManifestKind, ManifestData> = HashMap::new();
    web.insert(ManifestKind::Npm, manifest_with(&["lodash"]));
    ctx.by_package.insert(5, web);
    let mut e2e: HashMap<ManifestKind, ManifestData> = HashMap::new();
    e2e.insert(ManifestKind::Npm, manifest_with(&["playwright"]));
    ctx.by_package.insert(6, e2e);

    let deps = DeclaredDeps::snapshot(&ctx);
    assert!(deps.contains(Some(5), "lodash"));
    assert!(!deps.contains(Some(5), "playwright"), "isolation: web must not see e2e deps");
    assert!(deps.contains(Some(6), "playwright"));
    // A package id absent from the per-package map falls back to the union,
    // exactly like `manifests_for`.
    assert!(deps.contains(Some(999), "playwright"));
    // A file outside every package sees the union.
    assert!(deps.contains(None, "playwright"));
}
