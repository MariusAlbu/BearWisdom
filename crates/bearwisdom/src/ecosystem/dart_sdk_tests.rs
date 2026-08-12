use super::*;

#[test]
fn ecosystem_identity() {
    let e = DartSdkEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["dart"]);
}

#[test]
fn activation_is_language_present() {
    let e = DartSdkEcosystem;
    assert!(matches!(
        e.activation(),
        EcosystemActivation::LanguagePresent("dart")
    ));
}

#[test]
fn supports_reachability_and_demand_driven() {
    let e = DartSdkEcosystem;
    assert!(Ecosystem::supports_reachability(&e));
    assert!(Ecosystem::uses_demand_driven_parse(&e));
}

#[test]
fn locate_roots_empty_on_missing_sdk() {
    // Must not panic when Dart SDK is absent.
    let e = DartSdkEcosystem;
    let _ = Ecosystem::locate_roots(
        &e,
        &LocateContext {
            project_root: std::path::Path::new("."),
            manifests: &Default::default(),
            active_ecosystems: &[],
        },
    );
}

#[test]
fn walk_root_empty_on_bogus_dep() {
    let dep = ExternalDepRoot {
        module_path: "dart-sdk".to_string(),
        version: String::new(),
        root: PathBuf::from("/nonexistent/dart/sdk/lib"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let e = DartSdkEcosystem;
    assert!(Ecosystem::walk_root(&e, &dep).is_empty());
}

#[test]
fn dart_sdk_libs_are_nonempty() {
    assert!(!DART_SDK_LIBS.is_empty());
    assert!(DART_SDK_LIBS.contains(&"core"));
    assert!(DART_SDK_LIBS.contains(&"async"));
}
