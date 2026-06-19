// Tests for Swift module classification after the third-party-package purge.
// Apple platform-SDK frameworks stay in `is_external_swift_module`; SwiftPM
// package modules are classified from `Package.swift` deps at the resolver
// hooks via `manifest_dep_match` (case-insensitive, `swift-` prefix tolerant).

use super::predicates::{is_external_swift_module, manifest_dep_match};
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;

fn ctx_with_spm(deps: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut spm = ManifestData::default();
    for d in deps {
        spm.dependencies.insert((*d).to_string());
    }
    ctx.manifests.insert(ManifestKind::SwiftPM, spm);
    ctx
}

#[test]
fn spm_declared_package_classifies_external() {
    let ctx = ctx_with_spm(&["Alamofire", "RxSwift", "swift-nio"]);
    assert!(manifest_dep_match(Some(&ctx), "Alamofire"));
    // Case-insensitive.
    assert!(manifest_dep_match(Some(&ctx), "rxswift"));
    // `swift-` prefix tolerance: dep `swift-nio` matches module `nio`.
    assert!(manifest_dep_match(Some(&ctx), "nio"));
}

#[test]
fn package_without_manifest_is_not_external() {
    // No SwiftPM manifest → SwiftPM package modules no longer short-circuit
    // through the predicate (honest-unresolved without an install).
    assert!(!is_external_swift_module("Alamofire"));
    assert!(!is_external_swift_module("RxSwift"));
    assert!(!is_external_swift_module("Vapor"));
    assert!(!is_external_swift_module("Firebase"));
    assert!(!is_external_swift_module("Quick"));

    // Empty manifest present but the package isn't declared → not external.
    let ctx = ctx_with_spm(&[]);
    assert!(!manifest_dep_match(Some(&ctx), "Alamofire"));
}

#[test]
fn platform_frameworks_still_classify() {
    // Apple platform SDK (ships with Xcode) + Swift toolchain modules.
    assert!(is_external_swift_module("Foundation"));
    assert!(is_external_swift_module("UIKit"));
    assert!(is_external_swift_module("SwiftUI"));
    assert!(is_external_swift_module("Combine"));
    assert!(is_external_swift_module("CoreData"));
    assert!(is_external_swift_module("XCTest"));
    assert!(is_external_swift_module("Swift"));
    assert!(is_external_swift_module("Dispatch"));
    assert!(is_external_swift_module("Darwin"));
}

#[test]
fn purged_packages_absent_from_predicate() {
    for name in [
        "Vapor",
        "Fluent",
        "Leaf",
        "Queues",
        "JWT",
        "RxSwift",
        "RxCocoa",
        "Alamofire",
        "Moya",
        "SnapKit",
        "Kingfisher",
        "SDWebImage",
        "RealmSwift",
        "Realm",
        "Firebase",
        "FirebaseFirestore",
        "FirebaseAuth",
        "FirebaseStorage",
        "Quick",
        "Nimble",
    ] {
        assert!(
            !is_external_swift_module(name),
            "SwiftPM package `{name}` survived the purge"
        );
    }
}
