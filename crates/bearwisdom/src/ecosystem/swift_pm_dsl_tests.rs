use super::*;
use std::fs;

#[test]
fn manifest_api_under_modern_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let toolchain = tmp.path().to_path_buf();
    let manifest = toolchain.join("usr/lib/swift/pm/ManifestAPI");
    fs::create_dir_all(&manifest).unwrap();
    let found = manifest_api_under(&toolchain).expect("modern layout");
    assert_eq!(found, manifest);
}

#[test]
fn manifest_api_under_linux_alt_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let toolchain = tmp.path().to_path_buf();
    let manifest = toolchain.join("lib/swift/pm/ManifestAPI");
    fs::create_dir_all(&manifest).unwrap();
    let found = manifest_api_under(&toolchain).expect("alt layout");
    assert_eq!(found, manifest);
}

#[test]
fn manifest_api_under_windows_static_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let toolchain = tmp.path().to_path_buf();
    let manifest = toolchain.join("usr/lib/swift_static/pm/ManifestAPI");
    fs::create_dir_all(&manifest).unwrap();
    let found = manifest_api_under(&toolchain).expect("static layout");
    assert_eq!(found, manifest);
}

#[test]
fn manifest_api_under_returns_none_when_missing() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(manifest_api_under(tmp.path()).is_none());
}

#[test]
fn walk_manifest_api_picks_only_public_swiftinterface() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest_dir = tmp.path().join("ManifestAPI");
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(
        manifest_dir.join("PackageDescription.swiftinterface"),
        "// swift-interface-format-version: 1.0\npublic struct Package {}\n",
    )
    .unwrap();
    fs::write(
        manifest_dir.join("PackageDescription.private.swiftinterface"),
        "// private — ignored\n",
    )
    .unwrap();
    fs::write(manifest_dir.join("PackageDescription.swiftmodule"), "binary").unwrap();

    let dep = ExternalDepRoot {
        module_path: "PackageDescription".into(),
        version: String::new(),
        root: manifest_dir.clone(),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_manifest_api(&dep);
    assert_eq!(walked.len(), 1, "expected only public .swiftinterface; got {walked:?}");
    let file_name = walked[0]
        .absolute_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap();
    assert_eq!(file_name, "PackageDescription.swiftinterface");
    assert_eq!(walked[0].language, "swift");
    assert!(walked[0].relative_path.starts_with("ext:swift-pm-dsl:"));
}

#[test]
fn walk_returns_empty_when_dir_missing() {
    let dep = ExternalDepRoot {
        module_path: "PackageDescription".into(),
        version: String::new(),
        root: PathBuf::from("/__definitely_not_a_real_dir__/abc"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    assert!(walk_manifest_api(&dep).is_empty());
}
