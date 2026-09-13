use super::*;

use tempfile::TempDir;

fn write(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, content).unwrap();
}

/// A two-module Gradle workspace: `app` gets `app_build`, `core` is a plain
/// JVM library that names the AGP namespace only in a dependency exclusion.
fn workspace(app_build: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(
        root,
        "settings.gradle.kts",
        "include(\":app\", \":core\")\n",
    );
    write(root, "app/build.gradle.kts", app_build);
    write(
        root,
        "core/build.gradle.kts",
        "plugins {\n    id(\"java-library\")\n}\n\
         dependencies {\n    implementation(\"org.ow2.asm:asm:9.7\") {\n        \
         exclude(group = \"com.android.tools\")\n    }\n}\n",
    );
    tmp
}

fn modules(tmp: &TempDir) -> Vec<AndroidModule> {
    scan_android_modules(tmp.path(), tmp.path())
}

#[test]
fn a_plain_jvm_workspace_declares_no_android_module() {
    let tmp = workspace("plugins {\n    id(\"java-library\")\n}\n");
    assert!(modules(&tmp).is_empty());
}

#[test]
fn the_agp_namespace_plugin_declares_an_android_module() {
    let tmp = workspace(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdk = 35\n}\n",
    );
    let found = modules(&tmp);
    assert_eq!(found.len(), 1, "only app is an Android module: {found:?}");
    assert_eq!(found[0].package_dir, tmp.path().join("app"));
    assert_eq!(found[0].plugin_ids, vec!["com.android.application"]);
    assert_eq!(found[0].compile_sdk, Some(35));
}

#[test]
fn a_catalog_plugin_alias_declares_an_android_module() {
    let tmp = workspace(
        "plugins {\n    alias(libs.plugins.android.application)\n}\n\
         android {\n    compileSdk = libs.versions.android.compileSdk.get().toInt()\n}\n",
    );
    write(
        tmp.path(),
        "gradle/libs.versions.toml",
        "[versions]\nandroid-compileSdk = \"34\"\n\n\
         [plugins]\nandroid-application = { id = \"com.android.application\" }\n",
    );
    let found = modules(&tmp);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].plugin_ids, vec!["com.android.application"]);
    assert_eq!(found[0].compile_sdk, Some(34));
}

#[test]
fn a_subproject_reads_the_catalog_its_build_root_declares() {
    let tmp = workspace(
        "plugins {\n    alias(libs.plugins.android.library)\n}\n\
         android {\n    compileSdk = 33\n}\n",
    );
    write(
        tmp.path(),
        "gradle/libs.versions.toml",
        "[plugins]\nandroid-library = { id = \"com.android.library\" }\n",
    );
    // Scanning the subproject alone still resolves the alias, because the
    // catalog belongs to the settings file above it.
    let found = scan_android_modules(tmp.path(), &tmp.path().join("app"));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].plugin_ids, vec!["com.android.library"]);
    assert_eq!(found[0].compile_sdk, Some(33));
}

#[test]
fn the_extension_block_with_a_pin_declares_an_android_module() {
    // AGP applied through a convention plugin: no id in the namespace, but
    // the extension it installs is configured here.
    let tmp = workspace(
        "plugins {\n    id(\"myproject.android-conventions\")\n}\n\
         android {\n    compileSdk = 34\n}\n",
    );
    let found = modules(&tmp);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].plugin_ids.is_empty());
    assert_eq!(found[0].compile_sdk, Some(34));
}

#[test]
fn the_extension_block_with_a_component_manifest_declares_an_android_module() {
    let tmp = workspace(
        "plugins {\n    id(\"myproject.android-conventions\")\n}\n\
         android {\n    namespace = \"com.example.app\"\n}\n",
    );
    write(
        tmp.path(),
        "app/src/main/AndroidManifest.xml",
        "<manifest package=\"com.example.app\" />\n",
    );
    let found = modules(&tmp);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].compile_sdk, None);
}

#[test]
fn the_extension_block_alone_declares_nothing() {
    let tmp = workspace(
        "plugins {\n    id(\"myproject.android-conventions\")\n}\n\
         android {\n    namespace = \"com.example.app\"\n}\n",
    );
    assert!(modules(&tmp).is_empty());
}

#[test]
fn a_component_manifest_alone_declares_nothing() {
    let tmp = workspace("plugins {\n    id(\"java-library\")\n}\n");
    write(
        tmp.path(),
        "app/src/androidMain/AndroidManifest.xml",
        "<manifest package=\"com.example.app\" />\n",
    );
    assert!(modules(&tmp).is_empty());
}

#[test]
fn a_differently_named_extension_is_not_the_android_one() {
    let tmp = workspace(
        "plugins {\n    id(\"myproject.multiplatform\")\n}\n\
         kotlin {\n    optionalAndroid {\n        compileSdk = 36\n    }\n}\n\
         androidComponents {\n }\n",
    );
    write(
        tmp.path(),
        "app/src/androidMain/AndroidManifest.xml",
        "<manifest package=\"com.example.app\" />\n",
    );
    assert!(modules(&tmp).is_empty());
}

#[test]
fn pinned_levels_are_sorted_and_deduplicated() {
    let module = |level: Option<u32>| AndroidModule {
        package_dir: PathBuf::from("m"),
        build_file: PathBuf::from("m/build.gradle"),
        plugin_ids: Vec::new(),
        compile_sdk: level,
    };
    let found = vec![
        module(Some(35)),
        module(None),
        module(Some(33)),
        module(Some(35)),
    ];
    assert_eq!(pinned_api_levels(&found), vec![33, 35]);
}

#[test]
fn the_reader_is_silent_for_a_plain_jvm_workspace() {
    let tmp = workspace("plugins {\n    id(\"java-library\")\n}\n");
    assert!(AndroidModuleManifest.read(tmp.path()).is_none());
    assert!(AndroidModuleManifest.read_all(tmp.path()).is_empty());
}

#[test]
fn the_reader_attributes_one_entry_to_the_android_module() {
    let tmp = workspace(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdk = 35\n}\n",
    );
    let entries = AndroidModuleManifest.read_all(tmp.path());
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].package_dir, tmp.path().join("app"));
    assert_eq!(
        entries[0].manifest_path,
        tmp.path().join("app").join("build.gradle.kts")
    );
    assert!(entries[0]
        .data
        .dependencies
        .contains("com.android.application"));
    assert_eq!(AndroidModuleManifest.kind(), ManifestKind::AndroidModule);
}
