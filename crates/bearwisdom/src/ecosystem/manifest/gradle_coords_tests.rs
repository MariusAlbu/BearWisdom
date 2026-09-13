use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{collect_gradle_coords, collect_gradle_coords_scoped};

fn write(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

/// A build whose catalog and convention plugin live at the settings root
/// while the only ordinary build file sits in a subproject.
fn seed(root: &Path) {
    write(
        root,
        "settings.gradle.kts",
        "includeBuild(\"build-logic\")\ninclude(\":app\")\n",
    );
    write(
        root,
        "gradle/libs.versions.toml",
        "[libraries]\nconveyor = { module = \"com.fakeconv:conveyor\", version = \"3.1.0\" }\nbelt = { module = \"com.fakeconv:belt\", version = \"1.0.0\" }\n",
    );
    write(
        root,
        "build-logic/build.gradle.kts",
        "plugins {\n    `kotlin-dsl`\n}\n",
    );
    write(
        root,
        "build-logic/src/main/kotlin/Conventions.kt",
        "dependencies {\n    implementation(libs.belt)\n}\n",
    );
    write(
        root,
        "app/build.gradle.kts",
        "dependencies {\n    implementation(libs.conveyor)\n}\n",
    );
}

fn artifacts(coords: &[super::MavenCoord]) -> Vec<&str> {
    let mut out: Vec<&str> = coords.iter().map(|c| c.artifact_id.as_str()).collect();
    out.sort_unstable();
    out
}

#[test]
fn a_subproject_resolves_catalog_accessors_against_its_build_root() {
    let ws = TempDir::new().unwrap();
    seed(ws.path());

    let coords = collect_gradle_coords_scoped(ws.path(), &ws.path().join("app"));
    assert_eq!(artifacts(&coords), vec!["belt", "conveyor"]);
    assert!(coords.iter().all(|c| c.group_id == "com.fakeconv"));
}

#[test]
fn a_subproject_read_as_its_own_build_sees_no_catalog() {
    let ws = TempDir::new().unwrap();
    seed(ws.path());

    let coords = collect_gradle_coords(&ws.path().join("app"));
    assert!(coords.is_empty(), "{coords:?}");
}

#[test]
fn the_whole_build_collects_every_declaration_once() {
    let ws = TempDir::new().unwrap();
    seed(ws.path());

    let coords = collect_gradle_coords(ws.path());
    assert_eq!(artifacts(&coords), vec!["belt", "conveyor"]);
}
