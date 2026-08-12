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
        EcosystemActivation::ManifestFieldContains {
            manifest_glob,
            field_path,
            value,
        } => {
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

#[test]
fn discover_sky_engine_ui_when_present() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let flutter_root = tmp.path();
    std::fs::create_dir_all(flutter_root.join("packages").join("flutter").join("lib")).unwrap();
    let sky_ui = flutter_root
        .join("bin")
        .join("cache")
        .join("pkg")
        .join("sky_engine")
        .join("lib")
        .join("ui");
    std::fs::create_dir_all(&sky_ui).unwrap();
    std::fs::write(sky_ui.join("painting.dart"), "class Color {}\n").unwrap();

    let saved = std::env::var_os("BEARWISDOM_FLUTTER_SDK");
    std::env::set_var("BEARWISDOM_FLUTTER_SDK", flutter_root);
    let roots = discover_flutter_sdk();
    match saved {
        Some(v) => std::env::set_var("BEARWISDOM_FLUTTER_SDK", v),
        None => std::env::remove_var("BEARWISDOM_FLUTTER_SDK"),
    }

    let sky_root = roots
        .iter()
        .find(|r| r.ecosystem == super::super::dart_sdk::LEGACY_ECOSYSTEM_TAG)
        .expect("sky_engine ui root discovered");
    assert_eq!(sky_root.root, sky_ui);
    assert_eq!(sky_root.module_path, "dart-sdk");
}

#[test]
fn walk_dep_root_sky_engine_ui_matches_dart_sdk_scheme() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let sky_lib = tmp.path().join("sky_engine").join("lib");
    let sky_ui = sky_lib.join("ui");
    std::fs::create_dir_all(&sky_ui).unwrap();
    std::fs::write(sky_ui.join("painting.dart"), "class Color {}\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "dart-sdk".to_string(),
        version: String::new(),
        root: sky_ui,
        ecosystem: super::super::dart_sdk::LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_dep_root(&dep);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "ext:dart-sdk:ui/painting.dart");
    assert_eq!(files[0].language, "dart");
}

#[test]
fn walk_dep_root_flutter_package_still_uses_flutter_sdk_scheme() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let flutter_lib = tmp.path().join("lib");
    let src = flutter_lib.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("framework.dart"), "class Widget {}\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "flutter".to_string(),
        version: String::new(),
        root: flutter_lib,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_dep_root(&dep);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].relative_path, "ext:flutter-sdk:flutter/src/framework.dart");
}
