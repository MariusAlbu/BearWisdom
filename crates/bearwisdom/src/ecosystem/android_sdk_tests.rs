use super::*;

use std::sync::Mutex;

use tempfile::TempDir;

use crate::ecosystem::manifest::{read_all_manifests, ManifestKind};

/// The SDK-location variables are process-global; the tests that set them
/// must not overlap. Poison is recovered so a panicking test leaves a
/// usable lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const SDK_VARS: &[&str] = &["BEARWISDOM_ANDROID_SDK", "ANDROID_HOME", "ANDROID_SDK_ROOT"];

fn write(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, content).unwrap();
}

/// An SDK install holding the `sources/android-<N>` tree of each level.
fn seed_sdk(levels: &[u32]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    for level in levels {
        write(
            tmp.path(),
            &format!("sources/android-{level}/android/os/Bundle.java"),
            "package android.os;\npublic class Bundle {}\n",
        );
    }
    tmp
}

/// A single-module Gradle project whose build script is `build`.
fn seed_project(build: &str) -> TempDir {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "settings.gradle.kts", "include(\":app\")\n");
    write(tmp.path(), "app/build.gradle.kts", build);
    tmp
}

/// Discover roots with `sdk` as the only SDK location the locator can see.
fn roots_against(sdk: Option<&Path>, project: &Path) -> Vec<ExternalDepRoot> {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prior: Vec<(&str, Option<std::ffi::OsString>)> = SDK_VARS
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    for name in SDK_VARS {
        std::env::remove_var(name);
    }
    if let Some(sdk) = sdk {
        std::env::set_var("BEARWISDOM_ANDROID_SDK", sdk);
    }

    let roots = discover_android_sdk_roots(project, project);

    for (name, value) in prior {
        match value {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }
    roots
}

#[test]
fn ecosystem_identity() {
    let e = AndroidSdkEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["kotlin", "java"]);
    assert!(Ecosystem::supports_reachability(&e));
    assert!(Ecosystem::uses_demand_driven_parse(&e));
}

#[test]
fn activation_is_a_declared_android_module_on_a_jvm_build() {
    match AndroidSdkEcosystem.activation() {
        EcosystemActivation::All(clauses) => {
            assert_eq!(clauses.len(), 2, "{clauses:?}");
            match &clauses[0] {
                EcosystemActivation::TransitiveOn(id) => {
                    assert_eq!(*id, crate::ecosystem::maven::ID)
                }
                other => panic!("expected TransitiveOn, got {other:?}"),
            }
            assert!(matches!(&clauses[1], EcosystemActivation::ManifestMatch));
        }
        other => panic!("expected All, got {other:?}"),
    }
    assert_eq!(
        AndroidSdkEcosystem.manifest_kinds(),
        &[ManifestKind::AndroidModule]
    );
}

#[test]
fn activation_is_off_for_a_plain_gradle_project() {
    let project = seed_project(
        "plugins {\n    id(\"java-library\")\n}\n\
         dependencies {\n    implementation(\"org.ow2.asm:asm:9.7\") {\n        \
         exclude(group = \"com.android.tools\")\n    }\n}\n",
    );
    let manifests = read_all_manifests(project.path());
    assert!(
        !manifests.contains_key(&ManifestKind::AndroidModule),
        "a JVM build that only names the AGP namespace in an exclusion must not activate"
    );
}

#[test]
fn activation_is_on_for_a_module_applying_the_application_plugin() {
    let project = seed_project(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdk = 35\n}\n",
    );
    let manifests = read_all_manifests(project.path());
    assert!(manifests.contains_key(&ManifestKind::AndroidModule));
}

#[test]
fn activation_is_on_through_a_catalog_plugin_alias() {
    let project = seed_project(
        "plugins {\n    alias(libs.plugins.android.library)\n}\n\
         android {\n    compileSdk = 35\n}\n",
    );
    write(
        project.path(),
        "gradle/libs.versions.toml",
        "[plugins]\nandroid-library = { id = \"com.android.library\" }\n",
    );
    let manifests = read_all_manifests(project.path());
    assert!(manifests.contains_key(&ManifestKind::AndroidModule));
}

#[test]
fn a_project_without_an_android_module_gets_no_sdk() {
    let sdk = seed_sdk(&[35]);
    let project = seed_project("plugins {\n    id(\"java-library\")\n}\n");
    assert!(roots_against(Some(sdk.path()), project.path()).is_empty());
}

#[test]
fn the_compile_sdk_pin_selects_the_platform_directory() {
    let sdk = seed_sdk(&[33, 35]);
    let project = seed_project(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdk = 33\n}\n",
    );
    let roots = roots_against(Some(sdk.path()), project.path());
    assert_eq!(roots.len(), 1, "{roots:?}");
    assert_eq!(roots[0].root, sdk.path().join("sources").join("android-33"));
    assert_eq!(roots[0].module_path, "android-sdk:33");
    assert_eq!(roots[0].version, "33");
}

#[test]
fn a_platform_the_install_lacks_yields_nothing() {
    let sdk = seed_sdk(&[35]);
    let project = seed_project(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdkVersion(29)\n}\n",
    );
    assert!(
        roots_against(Some(sdk.path()), project.path()).is_empty(),
        "a pinned platform that is not installed must not fall back to another"
    );
}

#[test]
fn an_unpinned_module_falls_back_to_the_newest_installed_platform() {
    let sdk = seed_sdk(&[33, 35]);
    let project = seed_project(
        "plugins {\n    id(\"com.android.library\")\n}\n\
         android {\n    namespace = \"com.example.lib\"\n}\n",
    );
    let roots = roots_against(Some(sdk.path()), project.path());
    assert_eq!(roots.len(), 1, "{roots:?}");
    assert_eq!(roots[0].root, sdk.path().join("sources").join("android-35"));
}

#[test]
fn an_android_module_without_an_sdk_install_yields_nothing() {
    let project = seed_project(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdk = 35\n}\n",
    );
    assert!(roots_against(None, project.path()).is_empty());
}

#[test]
fn a_package_scoped_discovery_stamps_the_declaring_package() {
    let sdk = seed_sdk(&[35]);
    let project = seed_project(
        "plugins {\n    id(\"com.android.application\")\n}\n\
         android {\n    compileSdk = 35\n}\n",
    );
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prior: Vec<(&str, Option<std::ffi::OsString>)> = SDK_VARS
        .iter()
        .map(|name| (*name, std::env::var_os(name)))
        .collect();
    for name in SDK_VARS {
        std::env::remove_var(name);
    }
    std::env::set_var("BEARWISDOM_ANDROID_SDK", sdk.path());

    let app = project.path().join("app");
    let roots = ExternalSourceLocator::locate_roots_for_package(
        &AndroidSdkEcosystem,
        project.path(),
        &app,
        7,
    );
    let core_roots = ExternalSourceLocator::locate_roots_for_package(
        &AndroidSdkEcosystem,
        project.path(),
        &project.path().join("core"),
        8,
    );

    for (name, value) in prior {
        match value {
            Some(v) => std::env::set_var(name, v),
            None => std::env::remove_var(name),
        }
    }

    assert_eq!(roots.len(), 1, "{roots:?}");
    assert_eq!(roots[0].package_id, Some(7));
    assert!(
        core_roots.is_empty(),
        "a package that declares no Android module gets no SDK root"
    );
}

#[test]
fn walk_root_is_empty_for_a_missing_root() {
    let dep = ExternalDepRoot {
        module_path: "android-sdk:35".to_string(),
        version: "35".to_string(),
        root: PathBuf::from("/nonexistent/sources/android-35"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    assert!(Ecosystem::walk_root(&AndroidSdkEcosystem, &dep).is_empty());
}
