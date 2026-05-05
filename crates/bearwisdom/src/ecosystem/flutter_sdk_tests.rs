use super::*;

#[test]
fn ecosystem_identity() {
    let e = FlutterSdkEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["dart"]);
}

#[test]
fn activation_is_pubspec_flutter_dep() {
    let e = FlutterSdkEcosystem;
    match e.activation() {
        EcosystemActivation::ManifestFieldContains { manifest_glob, field_path, value } => {
            assert_eq!(manifest_glob, "**/pubspec.yaml");
            assert_eq!(field_path, "dependencies");
            assert_eq!(value, "flutter");
        }
        other => panic!("expected ManifestFieldContains, got {:?}", other),
    }
}

#[test]
fn supports_reachability_and_demand_driven() {
    let e = FlutterSdkEcosystem;
    assert!(Ecosystem::supports_reachability(&e));
    assert!(Ecosystem::uses_demand_driven_parse(&e));
}

#[test]
fn locate_roots_empty_on_missing_sdk() {
    // Must not panic when Flutter SDK is absent.
    let e = FlutterSdkEcosystem;
    let _ = Ecosystem::locate_roots(&e, &LocateContext {
        project_root: std::path::Path::new("."),
        manifests: &Default::default(),
        active_ecosystems: &[],
    });
}

#[test]
fn walk_root_empty_on_bogus_dep() {
    let dep = ExternalDepRoot {
        module_path: "flutter".to_string(),
        version: String::new(),
        root: PathBuf::from("/nonexistent/flutter/packages/flutter/lib"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let e = FlutterSdkEcosystem;
    assert!(Ecosystem::walk_root(&e, &dep).is_empty());
}

#[test]
fn extra_packages_list_is_nonempty() {
    assert!(!EXTRA_FLUTTER_PACKAGES.is_empty());
    assert!(EXTRA_FLUTTER_PACKAGES.contains(&"flutter_test"));
}

#[test]
fn flutter_sdk_id_differs_from_dart_sdk() {
    use super::super::dart_sdk;
    assert_ne!(ID, dart_sdk::ID);
}
