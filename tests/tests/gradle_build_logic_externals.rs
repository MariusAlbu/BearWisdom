//! Integration test for JVM externals declared in Gradle build logic.
//!
//! A Gradle dependency can be declared in a convention plugin
//! (`build-logic/src/main/kotlin/*.kt`) rather than in any
//! `build.gradle[.kts]`. The externals locator must discover that
//! declaration, resolve it against the Gradle dependency cache, and land the
//! artifact's sources like any other coordinate.
//!
//! Seeds a fake Gradle cache in the real hash-bucketed layout
//! `<group>/<artifact>/<version>/<hash>/<artifact>-<version>-sources.jar`
//! (jar assembled in memory with the zip crate), points
//! `BEARWISDOM_GRADLE_CACHE` at it, hides the machine's JDK and Kotlin
//! installs, and indexes a tiny project. The negative
//! control declares the same dependency in an ordinary `build.gradle.kts`, so
//! the only variable between the two is which file holds the declaration.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use bearwisdom::full_index;
use bearwisdom_tests::TestProject;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

/// The cache env vars are process-global; tests that set them must not run
/// concurrently. Poison is recovered so a panicking test leaves a usable lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

const GROUP: &str = "com.fakeconv";
const ARTIFACT: &str = "conveyor";
const VERSION: &str = "3.1.0";

/// Anchor TempDir holding an empty `m2` (so the extraction cache is derived
/// per-test instead of from the machine's shared one) and a `gradle-cache`
/// seeded with one `-sources.jar` for `com.fakeconv:conveyor:3.1.0`.
fn seed_isolated_caches() -> TempDir {
    let anchor = TempDir::new().unwrap();
    fs::create_dir_all(anchor.path().join("m2")).unwrap();

    let hash_dir = anchor
        .path()
        .join("gradle-cache")
        .join(GROUP)
        .join(ARTIFACT)
        .join(VERSION)
        .join("0123456789abcdef0123456789abcdef01234567");
    fs::create_dir_all(&hash_dir).unwrap();

    let jar_path = hash_dir.join(format!("{ARTIFACT}-{VERSION}-sources.jar"));
    let jar_file = fs::File::create(&jar_path).unwrap();
    let mut zip = zip::ZipWriter::new(jar_file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("com/fakeconv/conveyor/Conveyor.java", options)
        .unwrap();
    zip.write_all(
        br#"package com.fakeconv.conveyor;

/** A trivial conveyor. */
public class Conveyor {
    public String convey(String payload) {
        return payload;
    }
}
"#,
    )
    .unwrap();
    zip.finish().unwrap();

    anchor
}

/// The catalog entry and the user file are identical in both project shapes;
/// only the file holding `implementation(libs.conveyor)` differs.
fn seed_project(declaration_in_build_logic: bool) -> TestProject {
    let project = TestProject {
        dir: TempDir::new().unwrap(),
    };

    project.add_file("settings.gradle.kts", "includeBuild(\"build-logic\")\n");
    project.add_file(
        "gradle/libs.versions.toml",
        &format!(
            "[libraries]\nconveyor = {{ module = \"{GROUP}:{ARTIFACT}\", version = \"{VERSION}\" }}\n"
        ),
    );
    project.add_file(
        "build-logic/build.gradle.kts",
        "plugins {\n    `kotlin-dsl`\n}\n",
    );

    let declaration = "dependencies {\n    implementation(libs.conveyor)\n}\n";
    if declaration_in_build_logic {
        project.add_file("app/build.gradle.kts", "plugins {\n}\n");
        project.add_file("build-logic/src/main/kotlin/Conventions.kt", declaration);
    } else {
        project.add_file("app/build.gradle.kts", declaration);
    }

    project.add_file(
        "app/src/main/java/example/consumer/App.java",
        r#"package example.consumer;

import com.fakeconv.conveyor.Conveyor;

public class App {
    public void run(Conveyor c) {
        c.convey("payload");
    }
}
"#,
    );

    project
}

/// Environment the index runs under: both JVM dependency caches point at
/// `anchor`, and the JDK / Kotlin toolchain homes point at a directory
/// holding no sources, so the machine's stdlib installs never join the
/// fixture — the only external declaration in play is the seeded one.
fn isolated_environment(anchor: &Path) -> Vec<(&'static str, std::path::PathBuf)> {
    vec![
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
fn index_with_seeded_caches(anchor: &Path, project: &TestProject) -> bearwisdom::Database {
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

/// Number of external files whose path names the seeded class.
fn external_conveyor_files(db: &bearwisdom::Database) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM files
         WHERE origin = 'external' AND path LIKE '%Conveyor.java'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

/// Number of edges pointing at the external `convey` method.
fn convey_edges(db: &bearwisdom::Database) -> i64 {
    db.query_row(
        "SELECT COUNT(*) FROM edges e
         JOIN symbols s ON s.id = e.target_id
         WHERE s.origin = 'external' AND s.name = 'convey'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn dependency_declared_only_in_a_convention_plugin_lands_its_sources() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_isolated_caches();
    let project = seed_project(true);

    let db = index_with_seeded_caches(anchor.path(), &project);

    let files = external_conveyor_files(&db);
    assert!(
        files >= 1,
        "convention-plugin declaration did not pull Conveyor.java ({files})"
    );
    let edges = convey_edges(&db);
    assert!(
        edges >= 1,
        "user call did not resolve to the external convey method ({edges})"
    );
}

#[test]
fn dependency_declared_in_a_build_file_lands_its_sources() {
    let _env = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let anchor = seed_isolated_caches();
    let project = seed_project(false);

    let db = index_with_seeded_caches(anchor.path(), &project);

    let files = external_conveyor_files(&db);
    assert!(
        files >= 1,
        "build.gradle.kts declaration did not pull Conveyor.java ({files})"
    );
    let edges = convey_edges(&db);
    assert!(
        edges >= 1,
        "user call did not resolve to the external convey method ({edges})"
    );
}
