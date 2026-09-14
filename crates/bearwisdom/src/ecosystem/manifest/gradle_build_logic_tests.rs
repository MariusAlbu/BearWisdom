use std::fs;
use std::path::Path;

use tempfile::TempDir;

use super::{collect_gradle_build_logic_files, parse_included_build_paths};
use crate::ecosystem::manifest::gradle_coords::collect_gradle_coords;

/// Write `content` at `rel` under `root`, creating parent directories.
fn write(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

/// True when some collected path ends with `suffix` (slash-normalized).
fn contains_suffix(paths: &[std::path::PathBuf], suffix: &str) -> bool {
    paths
        .iter()
        .any(|p| p.to_string_lossy().replace('\\', "/").ends_with(suffix))
}

/// An included build that publishes Gradle plugins: its `src/main` holds
/// convention-plugin classes.
fn seed_plugin_development_build(root: &Path) {
    write(
        root,
        "settings.gradle.kts",
        "includeBuild(\"build-logic\")\n",
    );
    write(
        root,
        "build-logic/build.gradle.kts",
        "plugins {\n    `kotlin-dsl`\n}\n",
    );
    write(
        root,
        "build-logic/src/main/kotlin/conv/CommonConfig.kt",
        "dependencies {\n    api(libs.kotlinx.coroutines.core)\n}\n",
    );
}

#[test]
fn included_build_paths_parsed_from_both_dsl_forms() {
    let content = r#"
rootProject.name = "x"
includeBuild("build-logic")
includeBuild '../build-settings-logic'
include(":app")
"#;
    let paths = parse_included_build_paths(content);
    assert!(paths.contains(&"build-logic".to_string()));
    assert!(paths.contains(&"../build-settings-logic".to_string()));
    assert!(
        !paths.iter().any(|p| p.contains("app")),
        "subproject include() was mistaken for an included build: {paths:?}"
    );
}

#[test]
fn plugin_development_build_contributes_convention_sources() {
    let tmp = TempDir::new().unwrap();
    seed_plugin_development_build(tmp.path());

    let files = collect_gradle_build_logic_files(tmp.path());
    assert!(
        contains_suffix(&files, "build-logic/src/main/kotlin/conv/CommonConfig.kt"),
        "convention-plugin source not collected: {files:?}"
    );
}

#[test]
fn plain_included_build_contributes_only_precompiled_scripts() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "settings.gradle.kts", "includeBuild(\"tool\")\n");
    write(
        root,
        "tool/build.gradle.kts",
        "plugins {\n    kotlin(\"jvm\")\n}\n",
    );
    write(root, "tool/src/main/kotlin/App.kt", "fun main() {}\n");
    write(
        root,
        "tool/src/main/kotlin/conv.gradle.kts",
        "dependencies {\n    implementation(libs.greeter)\n}\n",
    );

    let files = collect_gradle_build_logic_files(root);
    assert!(
        contains_suffix(&files, "tool/src/main/kotlin/conv.gradle.kts"),
        "precompiled script plugin not collected: {files:?}"
    );
    assert!(
        !contains_suffix(&files, "tool/src/main/kotlin/App.kt"),
        "application source swept in from a non-plugin build: {files:?}"
    );
}

#[test]
fn build_src_is_a_build_logic_root_without_a_settings_entry() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "settings.gradle.kts", "rootProject.name = \"x\"\n");
    write(
        root,
        "buildSrc/build.gradle.kts",
        "plugins {\n    `kotlin-dsl`\n}\n",
    );
    write(
        root,
        "buildSrc/src/main/kotlin/Conventions.kt",
        "fun configure() {}\n",
    );

    let files = collect_gradle_build_logic_files(root);
    assert!(
        contains_suffix(&files, "buildSrc/src/main/kotlin/Conventions.kt"),
        "implicit buildSrc build was not walked: {files:?}"
    );
}

#[test]
fn generated_plugin_blocks_under_build_are_pruned() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    seed_plugin_development_build(root);
    write(
        root,
        "build-logic/build/kotlin-dsl/plugins-blocks/extracted/conv.gradle.kts",
        "dependencies {}\n",
    );

    let files = collect_gradle_build_logic_files(root);
    assert!(
        !contains_suffix(&files, "plugins-blocks/extracted/conv.gradle.kts"),
        "Gradle's generated copy of a precompiled script was collected: {files:?}"
    );
}

#[test]
fn catalog_ref_declared_only_in_a_convention_plugin_yields_a_coord() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    seed_plugin_development_build(root);
    write(root, "app/build.gradle.kts", "plugins {\n}\n");
    write(
        root,
        "gradle/libs.versions.toml",
        r#"[versions]
coroutines = "1.10.2"

[libraries]
kotlinx-coroutines-core = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version.ref = "coroutines" }
"#,
    );

    let coords = collect_gradle_coords(root);
    let hit = coords
        .iter()
        .find(|c| c.artifact_id == "kotlinx-coroutines-core")
        .unwrap_or_else(|| {
            panic!("catalog ref in a convention plugin yielded no coord: {coords:?}")
        });
    assert_eq!(hit.group_id, "org.jetbrains.kotlinx");
    assert_eq!(hit.version.as_deref(), Some("1.10.2"));

    // Negative half: without the convention source the declaration is gone.
    fs::remove_file(root.join("build-logic/src/main/kotlin/conv/CommonConfig.kt")).unwrap();
    let coords = collect_gradle_coords(root);
    assert!(
        !coords
            .iter()
            .any(|c| c.artifact_id == "kotlinx-coroutines-core"),
        "coord survived removal of its only declaration: {coords:?}"
    );
}
