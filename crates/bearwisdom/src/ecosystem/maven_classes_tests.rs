use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{_test_discover_jars_in_caches, _test_resolve_jvm_bytecode_jar};
use crate::ecosystem::manifest::maven::MavenCoord;

// --- Fixture builders -------------------------------------------------------

/// Build a fake `~/.m2/repository` entry:
///   root/<group-as-path>/<artifact>/<version>/<file>
fn make_m2_entry(
    root: &Path,
    group: &str,
    artifact: &str,
    version: &str,
    file_name: &str,
) -> PathBuf {
    let mut dir = root.to_path_buf();
    for seg in group.split('.') {
        dir = dir.join(seg);
    }
    dir = dir.join(artifact).join(version);
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join(file_name);
    fs::write(&f, b"").unwrap();
    f
}

/// Build a fake `~/.gradle/caches/modules-2/files-2.1` entry:
///   root/<group>/<artifact>/<version>/<hash>/<file>
fn make_gradle_entry(
    root: &Path,
    group: &str,
    artifact: &str,
    version: &str,
    file_name: &str,
) -> PathBuf {
    let dir = root
        .join(group)
        .join(artifact)
        .join(version)
        .join("deadbeef");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join(file_name);
    fs::write(&f, b"").unwrap();
    f
}

/// Write a minimal `build.gradle` declaring one `implementation` coordinate.
fn write_build_gradle(project_root: &Path, coord: &str) {
    let content = format!("dependencies {{\n    implementation '{coord}'\n}}\n");
    fs::write(project_root.join("build.gradle"), content).unwrap();
}

fn coord(group: &str, artifact: &str, version: Option<&str>) -> MavenCoord {
    MavenCoord {
        group_id: group.to_string(),
        artifact_id: artifact.to_string(),
        version: version.map(str::to_string),
    }
}

// --- resolve_jvm_bytecode_jar ----------------------------------------------

#[test]
fn bytecode_jar_found_in_gradle_layout() {
    let gradle = TempDir::new().unwrap();
    let expected = make_gradle_entry(
        gradle.path(),
        "com.google.guava",
        "guava",
        "33.0.0-jre",
        "guava-33.0.0-jre.jar",
    );
    let c = coord("com.google.guava", "guava", Some("33.0.0-jre"));
    let jar = _test_resolve_jvm_bytecode_jar(None, Some(gradle.path()), None, &c);
    assert_eq!(jar.as_deref(), Some(expected.as_path()));
}

#[test]
fn bytecode_jar_found_in_m2_layout() {
    let m2 = TempDir::new().unwrap();
    let expected = make_m2_entry(
        m2.path(),
        "org.apache.commons",
        "commons-lang3",
        "3.14.0",
        "commons-lang3-3.14.0.jar",
    );
    let c = coord("org.apache.commons", "commons-lang3", Some("3.14.0"));
    let jar = _test_resolve_jvm_bytecode_jar(Some(m2.path()), None, None, &c);
    assert_eq!(jar.as_deref(), Some(expected.as_path()));
}

#[test]
fn bytecode_jar_picks_largest_version_when_unpinned() {
    let gradle = TempDir::new().unwrap();
    make_gradle_entry(gradle.path(), "org.example", "lib", "1.0.0", "lib-1.0.0.jar");
    let expected = make_gradle_entry(
        gradle.path(),
        "org.example",
        "lib",
        "2.3.0",
        "lib-2.3.0.jar",
    );
    // Version absent (dynamic / unresolved) → largest cached version wins.
    let c = coord("org.example", "lib", None);
    let jar = _test_resolve_jvm_bytecode_jar(None, Some(gradle.path()), None, &c);
    assert_eq!(jar.as_deref(), Some(expected.as_path()));
}

#[test]
fn bytecode_jar_falls_back_to_cached_version_when_pinned_version_absent() {
    // Manifest pins a version the cache doesn't hold (compile classpath
    // resolved a different one). The probe must fall back to a cached
    // version rather than dropping the coordinate entirely.
    let gradle = TempDir::new().unwrap();
    let expected = make_gradle_entry(
        gradle.path(),
        "jakarta.annotation",
        "jakarta.annotation-api",
        "2.1.1",
        "jakarta.annotation-api-2.1.1.jar",
    );
    // Pinned 3.0.0 is not in the cache — only 2.1.1 is.
    let c = coord("jakarta.annotation", "jakarta.annotation-api", Some("3.0.0"));
    let jar = _test_resolve_jvm_bytecode_jar(None, Some(gradle.path()), None, &c);
    assert_eq!(jar.as_deref(), Some(expected.as_path()));
}

#[test]
fn sources_jar_never_returned_as_bytecode_jar() {
    let gradle = TempDir::new().unwrap();
    // Only the -sources.jar is present — there is no bytecode jar to pick.
    make_gradle_entry(
        gradle.path(),
        "org.example",
        "lib",
        "1.0.0",
        "lib-1.0.0-sources.jar",
    );
    let c = coord("org.example", "lib", Some("1.0.0"));
    let jar = _test_resolve_jvm_bytecode_jar(None, Some(gradle.path()), None, &c);
    assert!(
        jar.is_none(),
        "a -sources.jar must never satisfy the bytecode-jar probe"
    );
}

// --- discover_jars_in_caches (coordinate-driven) ---------------------------

#[test]
fn declared_coord_with_bytecode_jar_is_discovered() {
    let project = TempDir::new().unwrap();
    write_build_gradle(project.path(), "com.google.guava:guava:33.0.0-jre");

    let gradle = TempDir::new().unwrap();
    let expected = make_gradle_entry(
        gradle.path(),
        "com.google.guava",
        "guava",
        "33.0.0-jre",
        "guava-33.0.0-jre.jar",
    );

    let jars = _test_discover_jars_in_caches(project.path(), None, Some(gradle.path()), None);
    assert_eq!(jars, vec![expected]);
}

#[test]
fn undeclared_artifact_in_cache_is_not_discovered() {
    // The load-bearing inversion: a jar present in the cache whose
    // coordinate the project never declares must NOT be picked up.
    let project = TempDir::new().unwrap();
    write_build_gradle(project.path(), "com.google.guava:guava:33.0.0-jre");

    let gradle = TempDir::new().unwrap();
    // Declared coord: present, should be discovered.
    let declared = make_gradle_entry(
        gradle.path(),
        "com.google.guava",
        "guava",
        "33.0.0-jre",
        "guava-33.0.0-jre.jar",
    );
    // Undeclared coord: present in the cache but absent from the manifest.
    make_gradle_entry(
        gradle.path(),
        "org.undeclared",
        "stray",
        "9.9.9",
        "stray-9.9.9.jar",
    );

    let jars = _test_discover_jars_in_caches(project.path(), None, Some(gradle.path()), None);
    assert_eq!(jars, vec![declared]);
}

#[test]
fn declared_coord_without_version_picks_largest_cached() {
    // build.gradle catalog/dynamic versions can leave coord.version absent;
    // the probe falls back to the largest cached version.
    let project = TempDir::new().unwrap();
    write_build_gradle(project.path(), "org.example:lib");

    let gradle = TempDir::new().unwrap();
    make_gradle_entry(gradle.path(), "org.example", "lib", "1.0.0", "lib-1.0.0.jar");
    let expected = make_gradle_entry(
        gradle.path(),
        "org.example",
        "lib",
        "2.0.0",
        "lib-2.0.0.jar",
    );

    let jars = _test_discover_jars_in_caches(project.path(), None, Some(gradle.path()), None);
    assert_eq!(jars, vec![expected]);
}

#[test]
fn declared_coord_with_cached_sources_jar_is_skipped() {
    // When the -sources.jar is cached, the source path indexes it; the
    // bytecode walker must skip the coordinate to avoid double-indexing.
    let project = TempDir::new().unwrap();
    write_build_gradle(project.path(), "org.example:lib:1.0.0");

    let gradle = TempDir::new().unwrap();
    make_gradle_entry(gradle.path(), "org.example", "lib", "1.0.0", "lib-1.0.0.jar");
    make_gradle_entry(
        gradle.path(),
        "org.example",
        "lib",
        "1.0.0",
        "lib-1.0.0-sources.jar",
    );

    let jars = _test_discover_jars_in_caches(project.path(), None, Some(gradle.path()), None);
    assert!(
        jars.is_empty(),
        "coordinate with a cached sources jar must be skipped, got {jars:?}"
    );
}

#[test]
fn workspace_own_module_coord_is_excluded() {
    // A multi-module build whose settings.gradle declares `:mylib` as a
    // subproject, while a sibling module depends on it by published
    // coordinate. The coordinate resolves to project build output, not a
    // cached jar — even if a stray jar with that name sits in the cache it
    // must NOT be probed. A genuine third-party coord in the same build is.
    let project = TempDir::new().unwrap();
    fs::write(
        project.path().join("settings.gradle"),
        "include(':mylib')\n",
    )
    .unwrap();
    let content = "dependencies {\n    \
         implementation 'com.example:mylib:1.0.0'\n    \
         implementation 'com.google.guava:guava:33.0.0-jre'\n}\n";
    fs::write(project.path().join("build.gradle"), content).unwrap();

    let gradle = TempDir::new().unwrap();
    // Own module: present in the cache, but must be skipped via exclusion.
    make_gradle_entry(
        gradle.path(),
        "com.example",
        "mylib",
        "1.0.0",
        "mylib-1.0.0.jar",
    );
    // Genuine external: must be discovered.
    let external = make_gradle_entry(
        gradle.path(),
        "com.google.guava",
        "guava",
        "33.0.0-jre",
        "guava-33.0.0-jre.jar",
    );

    let jars = _test_discover_jars_in_caches(project.path(), None, Some(gradle.path()), None);
    assert_eq!(
        jars,
        vec![external],
        "the workspace's own module coordinate must be excluded from the bytecode probe"
    );
}

#[test]
fn project_local_libs_jar_is_still_discovered() {
    // The project-scoped lib/libs/vendor/deps scan survives the conversion
    // to coordinate-driven cache probing.
    let project = TempDir::new().unwrap();
    let libs = project.path().join("libs");
    fs::create_dir_all(&libs).unwrap();
    let vendored = libs.join("vendored-1.0.0.jar");
    fs::write(&vendored, b"").unwrap();
    // A classifier jar in the same dir must be ignored.
    fs::write(libs.join("vendored-1.0.0-sources.jar"), b"").unwrap();

    let jars = _test_discover_jars_in_caches(project.path(), None, None, None);
    assert_eq!(jars, vec![vendored]);
}
