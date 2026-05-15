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
    assert!(!publishable, "publish = false must flip is_publishable to false");
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
    assert!(!publishable, "publish = [] must flip is_publishable to false");
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
    assert!(publishable, "explicit registry list must keep publishable=true");
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
    assert!(publishable, "publish=false in a non-[package] section must not flip the bit");
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
    write(
        tmp.path(),
        "package.json",
        r#"{"name": "@org/lib"}"#,
    );
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
    assert!(!publishable, "PEP 301 private classifier must flip is_publishable=false");
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
