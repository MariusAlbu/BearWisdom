use std::fs;

use tempfile::TempDir;

use super::gradle_build_root;

#[test]
fn a_subproject_belongs_to_the_settings_directory_above_it() {
    let ws = TempDir::new().unwrap();
    fs::create_dir_all(ws.path().join("app/src")).unwrap();
    fs::write(ws.path().join("settings.gradle.kts"), "include(\":app\")\n").unwrap();

    let root = gradle_build_root(ws.path(), &ws.path().join("app"));
    assert_eq!(root, ws.path());
}

#[test]
fn a_nested_build_with_its_own_settings_is_its_own_root() {
    let ws = TempDir::new().unwrap();
    let nested = ws.path().join("services/billing");
    fs::create_dir_all(nested.join("api")).unwrap();
    fs::write(ws.path().join("settings.gradle"), "").unwrap();
    fs::write(nested.join("settings.gradle"), "").unwrap();

    assert_eq!(gradle_build_root(ws.path(), &nested.join("api")), nested);
}

#[test]
fn without_a_settings_file_the_package_directory_is_the_root() {
    let ws = TempDir::new().unwrap();
    fs::create_dir_all(ws.path().join("lib")).unwrap();

    let lib = ws.path().join("lib");
    assert_eq!(gradle_build_root(ws.path(), &lib), lib);
}

#[test]
fn the_search_never_leaves_the_workspace() {
    let outer = TempDir::new().unwrap();
    fs::write(outer.path().join("settings.gradle.kts"), "").unwrap();
    let ws = outer.path().join("workspace");
    fs::create_dir_all(ws.join("app")).unwrap();

    assert_eq!(gradle_build_root(&ws, &ws.join("app")), ws.join("app"));
}
