use super::*;
use std::fs;
use tempfile::TempDir;

fn write(dir: &std::path::Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).unwrap();
}

// ---------------------------------------------------------------------------
// Cargo
// ---------------------------------------------------------------------------

#[test]
fn cargo_publish_false_is_not_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"internal-helper\"\npublish = false\n",
    );
    let (name, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert_eq!(name.as_deref(), Some("internal-helper"));
    assert!(
        !publishable,
        "publish = false must flip is_publishable to false"
    );
}

#[test]
fn cargo_publish_empty_array_is_not_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"x\"\npublish = []\n",
    );
    let (_, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert!(
        !publishable,
        "publish = [] must flip is_publishable to false"
    );
}

#[test]
fn cargo_publish_absent_defaults_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"public-crate\"\n",
    );
    let (name, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert_eq!(name.as_deref(), Some("public-crate"));
    assert!(publishable, "no publish key must default to true");
}

#[test]
fn cargo_publish_true_is_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"x\"\npublish = true\n",
    );
    let (_, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert!(publishable);
}

#[test]
fn cargo_publish_registry_list_is_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"x\"\npublish = [\"crates-io\"]\n",
    );
    let (_, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert!(
        publishable,
        "explicit registry list must keep publishable=true"
    );
}

#[test]
fn cargo_publish_in_section_not_outside_it() {
    // `publish` keys outside the `[package]` block (e.g. in `[workspace]`)
    // must not bleed into per-crate is_publishable.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "Cargo.toml",
        "[package]\nname = \"x\"\n\n[some.other.section]\npublish = false\n",
    );
    let (_, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert!(
        publishable,
        "publish=false in a non-[package] section must not flip the bit"
    );
}

// ---------------------------------------------------------------------------
// npm
// ---------------------------------------------------------------------------

#[test]
fn npm_private_true_is_not_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "package.json",
        r#"{"name": "@org/internal", "private": true}"#,
    );
    let (name, publishable) = read_package_manifest(tmp.path(), "npm");
    assert_eq!(name.as_deref(), Some("@org/internal"));
    assert!(!publishable);
}

#[test]
fn npm_private_false_is_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "package.json",
        r#"{"name": "@org/public", "private": false}"#,
    );
    let (_, publishable) = read_package_manifest(tmp.path(), "npm");
    assert!(publishable);
}

#[test]
fn npm_no_private_key_is_publishable() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "package.json", r#"{"name": "@org/lib"}"#);
    let (name, publishable) = read_package_manifest(tmp.path(), "npm");
    assert_eq!(name.as_deref(), Some("@org/lib"));
    assert!(publishable);
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

#[test]
fn pyproject_private_classifier_is_not_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pyproject.toml",
        r#"[project]
name = "internal-tool"
classifiers = [
    "Private :: Do Not Upload",
    "Programming Language :: Python :: 3",
]
"#,
    );
    let (name, publishable) = read_package_manifest(tmp.path(), "python");
    assert_eq!(name.as_deref(), Some("internal-tool"));
    assert!(
        !publishable,
        "PEP 301 private classifier must flip is_publishable=false"
    );
}

#[test]
fn pyproject_poetry_private_true_is_not_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pyproject.toml",
        r#"[tool.poetry]
name = "poetry-internal"
private = true
"#,
    );
    let (_, publishable) = read_package_manifest(tmp.path(), "python");
    assert!(!publishable);
}

#[test]
fn pyproject_default_is_publishable() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pyproject.toml",
        r#"[project]
name = "public-lib"
"#,
    );
    let (name, publishable) = read_package_manifest(tmp.path(), "python");
    assert_eq!(name.as_deref(), Some("public-lib"));
    assert!(publishable);
}

// ---------------------------------------------------------------------------
// Dart
// ---------------------------------------------------------------------------

#[test]
fn pubspec_name_is_declared_name() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pubspec.yaml",
        "name: ui_kit\ndescription: a widget library\ndependencies:\n  flutter:\n    sdk: flutter\n",
    );
    let (name, publishable) = read_package_manifest(tmp.path(), "dart");
    assert_eq!(name.as_deref(), Some("ui_kit"));
    assert!(publishable);
}

#[test]
fn pubspec_missing_name_is_none() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pubspec.yaml", "description: no name here\n");
    let (name, _) = read_package_manifest(tmp.path(), "dart");
    assert!(name.is_none());
}

// ---------------------------------------------------------------------------
// Cross-cutting
// ---------------------------------------------------------------------------

#[test]
fn missing_manifest_defaults_publishable() {
    // Empty directory with no manifest file. Function returns (None, true)
    // so the caller upserts a package row with is_publishable defaulting
    // to true — matches the v0 behavior for manifests we can't parse.
    let tmp = TempDir::new().unwrap();
    let (name, publishable) = read_package_manifest(tmp.path(), "cargo");
    assert!(name.is_none());
    assert!(publishable);
}

#[test]
fn unknown_kind_defaults_publishable() {
    let tmp = TempDir::new().unwrap();
    let (name, publishable) = read_package_manifest(tmp.path(), "lua");
    assert!(name.is_none());
    assert!(publishable);
}

// ---------------------------------------------------------------------------
// Workspace root: pure-controller vs hybrid root-as-app
// ---------------------------------------------------------------------------

#[test]
fn workspace_root_hybrid_with_runtime_deps_is_registered() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "package.json",
        r#"{
            "name": "vuestic-admin",
            "private": true,
            "workspaces": ["e2e"],
            "dependencies": { "vue": "^3.5.0", "pinia": "^2.0.0" },
            "devDependencies": { "vite": "^5.0.0" }
        }"#,
    );
    fs::create_dir_all(tmp.path().join("e2e")).unwrap();
    write(
        &tmp.path().join("e2e"),
        "package.json",
        r#"{"name":"e2e","private":true,"devDependencies":{"@playwright/test":"^1.54.2"}}"#,
    );

    let (packages, kind) = detect_packages(tmp.path());
    assert_eq!(kind.as_deref(), Some("npm-workspaces"));
    let paths: std::collections::HashSet<_> = packages.iter().map(|p| p.path.as_str()).collect();
    assert!(
        paths.contains("") || paths.contains("."),
        "hybrid root with own runtime deps must register as a package — saw {paths:?}"
    );
    assert!(
        paths.contains("e2e"),
        "workspace member e2e must also register"
    );
}

#[test]
fn workspace_root_pure_controller_with_dev_only_deps_skipped() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "package.json",
        r#"{
            "name": "monorepo-controller",
            "private": true,
            "workspaces": ["packages/*"],
            "devDependencies": { "turbo": "^2.0.0", "@changesets/cli": "^2.0.0" }
        }"#,
    );
    fs::create_dir_all(tmp.path().join("packages").join("app")).unwrap();
    write(
        &tmp.path().join("packages").join("app"),
        "package.json",
        r#"{"name":"app","dependencies":{"react":"^18.0.0"}}"#,
    );

    let (packages, kind) = detect_packages(tmp.path());
    assert_eq!(kind.as_deref(), Some("npm-workspaces"));
    let paths: std::collections::HashSet<_> = packages.iter().map(|p| p.path.as_str()).collect();
    assert!(
        !paths.contains("") && !paths.contains("."),
        "pure controller (devDeps only) must NOT register as a package — saw {paths:?}"
    );
    assert!(paths.contains("packages/app"));
}

#[test]
fn workspace_root_with_peer_deps_is_registered() {
    // peerDependencies signals a library root that's also a workspace
    // controller — still a real package whose deps must reach externals.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "package.json",
        r#"{
            "name": "lib-monorepo",
            "private": true,
            "workspaces": ["examples/*"],
            "peerDependencies": { "react": "^18.0.0" }
        }"#,
    );
    fs::create_dir_all(tmp.path().join("examples").join("demo")).unwrap();
    write(
        &tmp.path().join("examples").join("demo"),
        "package.json",
        r#"{"name":"demo"}"#,
    );

    let (packages, _) = detect_packages(tmp.path());
    let paths: std::collections::HashSet<_> = packages.iter().map(|p| p.path.as_str()).collect();
    assert!(
        paths.contains("") || paths.contains("."),
        "root with peerDependencies must register — saw {paths:?}"
    );
}
