use super::*;

#[test]
fn both_reader_apis_retain_package_addressed_module_configuration() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='demo'").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();
    let entries = read_all_manifests_per_package(dir.path());
    let union = read_all_manifests(dir.path());
    assert_eq!(union[&ManifestKind::Cargo].module_packages.len(), 1);
    assert_eq!(
        entries
            .iter()
            .find(|p| p.kind == ManifestKind::Cargo)
            .unwrap()
            .data
            .module_packages,
        union[&ManifestKind::Cargo].module_packages
    );
}
use std::fs;
use tempfile::TempDir;

fn write_file(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn names_by_kind<'a>(manifests: &'a [PackageManifest], kind: ManifestKind) -> Vec<&'a str> {
    let mut names: Vec<&str> = manifests
        .iter()
        .filter(|m| m.kind == kind)
        .map(|m| m.name.as_str())
        .collect();
    names.sort();
    names
}

#[test]
fn per_package_splits_monorepo_npm() {
    // Synthetic pnpm-style monorepo: 3 packages, each with its own deps.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(root, "package.json", r#"{"name":"root","private":true}"#);
    write_file(
        root,
        "packages/server/package.json",
        r#"{"name":"@app/server","dependencies":{"express":"4"}}"#,
    );
    write_file(
        root,
        "packages/web/package.json",
        r#"{"name":"@app/web","dependencies":{"react":"18"}}"#,
    );
    write_file(
        root,
        "packages/e2e/package.json",
        r#"{"name":"@app/e2e","devDependencies":{"playwright":"1"}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let npm: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Npm)
        .collect();

    // 4 package.json files → 4 entries (root counts).
    assert_eq!(npm.len(), 4, "expected 4 npm manifests, got {}", npm.len());

    let names = names_by_kind(&per_pkg, ManifestKind::Npm);
    assert_eq!(names, vec!["@app/e2e", "@app/server", "@app/web", "root"]);

    // Per-package dep isolation: server has express but NOT react or playwright.
    let server = npm.iter().find(|m| m.name == "@app/server").unwrap();
    assert!(server.data.dependencies.contains("express"));
    assert!(!server.data.dependencies.contains("react"));
    assert!(!server.data.dependencies.contains("playwright"));

    let web = npm.iter().find(|m| m.name == "@app/web").unwrap();
    assert!(web.data.dependencies.contains("react"));
    assert!(!web.data.dependencies.contains("express"));

    let e2e = npm.iter().find(|m| m.name == "@app/e2e").unwrap();
    assert!(e2e.data.dependencies.contains("playwright"));
    assert!(!e2e.data.dependencies.contains("react"));
}

#[test]
fn per_package_splits_cargo_workspace() {
    // Synthetic Cargo workspace with 2 member crates.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/a\", \"crates/b\"]\n",
    );
    write_file(
        root,
        "crates/a/Cargo.toml",
        "[package]\nname = \"crate-a\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
    );
    write_file(
        root,
        "crates/b/Cargo.toml",
        "[package]\nname = \"crate-b\"\nversion = \"0.1.0\"\n\n[dependencies]\ntokio = \"1\"\n",
    );

    let per_pkg = read_all_manifests_per_package(root);
    let cargo: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Cargo)
        .collect();

    // 3 Cargo.toml files (workspace root + 2 members).
    assert_eq!(cargo.len(), 3);

    let names = names_by_kind(&per_pkg, ManifestKind::Cargo);
    // Workspace root has no [package].name → falls back to dir name ("the temp dir's last segment").
    // We can't predict the temp dir's name, but crate-a and crate-b must be present.
    assert!(names.contains(&"crate-a"));
    assert!(names.contains(&"crate-b"));

    let a = cargo.iter().find(|m| m.name == "crate-a").unwrap();
    assert!(a.data.dependencies.contains("serde"));
    assert!(!a.data.dependencies.contains("tokio"));

    let b = cargo.iter().find(|m| m.name == "crate-b").unwrap();
    assert!(b.data.dependencies.contains("tokio"));
    assert!(!b.data.dependencies.contains("serde"));
}

#[test]
fn single_package_yields_one_entry_per_ecosystem() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "package.json",
        r#"{"name":"solo","dependencies":{"lodash":"4"}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let npm: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Npm)
        .collect();

    assert_eq!(npm.len(), 1);
    assert_eq!(npm[0].name, "solo");
    assert_eq!(npm[0].path, PathBuf::new());
    assert!(npm[0].data.dependencies.contains("lodash"));
}

#[test]
fn legacy_read_all_matches_union_of_per_package() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "packages/a/package.json",
        r#"{"name":"a","dependencies":{"foo":"1"}}"#,
    );
    write_file(
        root,
        "packages/b/package.json",
        r#"{"name":"b","dependencies":{"bar":"1"}}"#,
    );

    let legacy = read_all_manifests(root);
    let npm_data = legacy.get(&ManifestKind::Npm).expect("npm union present");

    // Union must contain both foo and bar.
    assert!(npm_data.dependencies.contains("foo"));
    assert!(npm_data.dependencies.contains("bar"));
}

#[test]
fn empty_project_returns_nothing() {
    let tmp = TempDir::new().unwrap();
    let per_pkg = read_all_manifests_per_package(tmp.path());
    assert!(per_pkg.is_empty());

    let legacy = read_all_manifests(tmp.path());
    assert!(legacy.is_empty());
}

#[test]
fn skips_node_modules_when_walking() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "package.json",
        r#"{"name":"host","dependencies":{"react":"18"}}"#,
    );
    // A sub-package.json inside node_modules must NOT be picked up.
    write_file(
        root,
        "node_modules/react/package.json",
        r#"{"name":"react","version":"18.0.0","dependencies":{"loose-envify":"1"}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let npm: Vec<&PackageManifest> = per_pkg
        .iter()
        .filter(|m| m.kind == ManifestKind::Npm)
        .collect();
    assert_eq!(npm.len(), 1);
    assert_eq!(npm[0].name, "host");
}

#[test]
fn package_path_is_relative_to_project_root() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        root,
        "packages/server/package.json",
        r#"{"name":"server","dependencies":{}}"#,
    );

    let per_pkg = read_all_manifests_per_package(root);
    let server = per_pkg
        .iter()
        .find(|m| m.name == "server")
        .expect("server manifest");

    assert_eq!(server.path, PathBuf::from("packages").join("server"));
}

#[test]
fn manifest_path_is_absolute() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write_file(root, "package.json", r#"{"name":"solo","dependencies":{}}"#);

    let per_pkg = read_all_manifests_per_package(root);
    let solo = per_pkg.iter().find(|m| m.name == "solo").unwrap();
    assert!(solo.manifest_path.is_absolute());
    assert!(solo.manifest_path.ends_with("package.json"));
}
