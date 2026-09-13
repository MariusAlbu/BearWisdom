//! The Android platform SDK reaches a project only when the project declares
//! an Android module, and only at the platform that module compiles against.
//!
//! The platform redefines the whole `java.*`/`javax.*` surface, so walking it
//! into a JVM build that has nothing to do with Android gives every core type
//! a second declaration and bare heads stop resolving. The evidence that
//! keeps it out is the Android Gradle Plugin's application: a build script
//! that merely names the `com.android.*` namespace in a dependency exclusion
//! declares no Android module.
//!
//! Seeds a fake SDK holding one platform's sources
//! (`sources/android-35/android/os/Bundle.java`), points
//! `BEARWISDOM_ANDROID_SDK` at it, hides the machine's SDK, JDK and Kotlin
//! installs, and indexes the same two-module Gradle workspace three ways:
//! the app applies the plugin, the app applies it but pins a platform the
//! install lacks, and the app applies a plain JVM plugin instead.

use std::fs;
use std::path::Path;
use std::sync::Mutex;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;

/// The SDK-location env vars are process-global; tests that set them must not
/// run concurrently. Poison is recovered so a panicking test leaves a usable
/// lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// The one platform the seeded install holds.
const INSTALLED_LEVEL: u32 = 35;

/// What `app/build.gradle.kts` declares. The three variants differ only in
/// that file.
enum AppBuild {
    /// Applies the Android application plugin and compiles against the
    /// installed platform.
    AndroidOnInstalledPlatform,
    /// Applies the plugin but compiles against a platform the install lacks.
    AndroidOnMissingPlatform,
    /// A plain JVM library that names the AGP namespace only in an exclusion.
    PlainJvm,
}

impl AppBuild {
    fn script(&self) -> String {
        match self {
            AppBuild::AndroidOnInstalledPlatform => format!(
                "plugins {{\n    id(\"com.android.application\")\n}}\n\n\
                 android {{\n    compileSdk = {INSTALLED_LEVEL}\n}}\n"
            ),
            AppBuild::AndroidOnMissingPlatform => {
                "plugins {\n    id(\"com.android.application\")\n}\n\n\
                 android {\n    compileSdk = 31\n}\n"
                    .to_string()
            }
            AppBuild::PlainJvm => "plugins {\n    id(\"java-library\")\n}\n\n\
                 dependencies {\n    implementation(\"org.ow2.asm:asm:9.7\") {\n        \
                 exclude(group = \"com.android.tools\")\n    }\n}\n"
                .to_string(),
        }
    }
}

/// Anchor holding an empty `m2` + `gradle-cache` (so the JVM dependency
/// caches resolve to nothing) and an `android-sdk` install whose only
/// platform ships the sources for one class.
fn seed_isolated_caches() -> TempDir {
    let anchor = TempDir::new().unwrap();
    fs::create_dir_all(anchor.path().join("m2")).unwrap();
    fs::create_dir_all(anchor.path().join("gradle-cache")).unwrap();

    let platform = anchor
        .path()
        .join("android-sdk")
        .join("sources")
        .join(format!("android-{INSTALLED_LEVEL}"))
        .join("android")
        .join("os");
    fs::create_dir_all(&platform).unwrap();
    fs::write(
        platform.join("Bundle.java"),
        r#"package android.os;

/** The platform's key-value parcel container. */
public class Bundle {
    public String getString(String key) {
        return null;
    }
}
"#,
    )
    .unwrap();

    anchor
}

/// A two-module Gradle workspace. `core` is always a plain JVM library; only
/// `app`'s build script varies.
fn seed_project(app_build: &AppBuild) -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file("settings.gradle.kts", "include(\":app\", \":core\")\n");
    project.add_file("app/build.gradle.kts", &app_build.script());
    project.add_file(
        "core/build.gradle.kts",
        "plugins {\n    id(\"java-library\")\n}\n",
    );
    project.add_file(
        "app/src/main/java/example/app/Screen.java",
        r#"package example.app;

import android.os.Bundle;

public class Screen {
    public String title(Bundle state) {
        return state.getString("title");
    }
}
"#,
    );
    project.add_file(
        "core/src/main/java/example/core/Registry.java",
        r#"package example.core;

public class Registry {
    public String name() {
        return "core";
    }
}
"#,
    );

    project
}

/// Environment the index runs under: the Android SDK is the seeded install,
/// the JVM dependency caches and toolchain homes point at a directory holding
/// no sources, so the machine's own installs never join the fixture.
fn isolated_environment(anchor: &Path) -> Vec<(&'static str, std::path::PathBuf)> {
    vec![
        ("BEARWISDOM_ANDROID_SDK", anchor.join("android-sdk")),
        ("ANDROID_HOME", anchor.to_path_buf()),
        ("ANDROID_SDK_ROOT", anchor.to_path_buf()),
        ("BEARWISDOM_JAVA_MAVEN_REPO", anchor.join("m2")),
        ("BEARWISDOM_GRADLE_CACHE", anchor.join("gradle-cache")),
        ("JAVA_HOME", anchor.to_path_buf()),
        ("KOTLIN_HOME", anchor.to_path_buf()),
        ("KOTLINC_HOME", anchor.to_path_buf()),
        ("KOTLIN_ROOT", anchor.to_path_buf()),
    ]
}

/// Index `project` under `isolated_environment`, restoring the prior
/// environment before returning.
fn index_with_seeded_sdk(anchor: &Path, project: &TestProject) -> bearwisdom::Database {
    let overrides = isolated_environment(anchor);
    let prior: Vec<(&'static str, Option<std::ffi::OsString>)> = overrides
        .iter()
        .map(|(name, _)| (*name, std::env::var_os(name)))
        .collect();

    // SAFETY: std::env::set_var is process-global. ENV_LOCK is held by the
    // caller for the whole set→index→restore window.
    unsafe {
        for (name, value) in &overrides {
            std::env::set_var(name, value);
        }
    }

    let mut db = TestProject::in_memory_db();
    let _ = full_index(&mut db, project.path(), None, None, None).unwrap();

    unsafe {
        for (name, value) in prior {
            match value {
                Some(v) => std::env::set_var(name, v),
                None => std::env::remove_var(name),
            }
        }
    }

    db
}

/// External files walked out of the seeded platform.
fn android_platform_files(db: &bearwisdom::Database) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM files
         WHERE origin = 'external' AND path LIKE '%android-35%'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// Edges pointing at a member of the platform's `Bundle`.
fn platform_member_edges(db: &bearwisdom::Database) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON s.id = e.target_id
         JOIN files f ON f.id = s.file_id
         WHERE s.origin = 'external'
           AND s.name = 'getString'
           AND f.path LIKE '%android-35%'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn a_declared_android_module_lands_the_platform_it_compiles_against() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_isolated_caches();
    let project = seed_project(&AppBuild::AndroidOnInstalledPlatform);

    let db = index_with_seeded_sdk(anchor.path(), &project);

    let files = android_platform_files(&db);
    assert!(
        files >= 1,
        "the declared platform did not land Bundle.java ({files})"
    );
    let edges = platform_member_edges(&db);
    assert!(
        edges >= 1,
        "the call did not resolve to the platform's getString ({edges})"
    );
}

#[test]
fn a_workspace_without_an_android_module_lands_no_platform_file() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_isolated_caches();
    let project = seed_project(&AppBuild::PlainJvm);

    let db = index_with_seeded_sdk(anchor.path(), &project);

    let files = android_platform_files(&db);
    assert_eq!(
        files, 0,
        "a JVM-only workspace pulled {files} Android platform file(s)"
    );
}

#[test]
fn a_platform_the_install_lacks_lands_no_substitute() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_isolated_caches();
    let project = seed_project(&AppBuild::AndroidOnMissingPlatform);

    let db = index_with_seeded_sdk(anchor.path(), &project);

    let files = android_platform_files(&db);
    assert_eq!(
        files, 0,
        "a module pinned to android-31 was served {files} file(s) from another platform"
    );
}
