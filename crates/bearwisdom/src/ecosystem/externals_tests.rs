use std::fs;

use tempfile::TempDir;

use super::{resolve_gradle_sources_jar};
use crate::ecosystem::manifest::maven::MavenCoord;

/// Build a fake `~/.gradle/caches/modules-2/files-2.1` layout under `root`:
///
///   root/<group>/<artifact>/<version>/<hash>/<file>
fn make_gradle_cache_entry(
    root: &std::path::Path,
    group: &str,
    artifact: &str,
    version: &str,
    file_name: &str,
) -> std::path::PathBuf {
    let dir = root.join(group).join(artifact).join(version).join("deadbeef");
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join(file_name);
    fs::write(&f, b"").unwrap();
    f
}

#[test]
fn resolve_gradle_sources_jar_finds_jar_with_explicit_version() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();
    let expected = make_gradle_cache_entry(
        cache,
        "org.assertj",
        "assertj-core",
        "3.27.7",
        "assertj-core-3.27.7-sources.jar",
    );

    let coord = MavenCoord {
        group_id: "org.assertj".to_string(),
        artifact_id: "assertj-core".to_string(),
        version: Some("3.27.7".to_string()),
    };
    let (resolved_version, jar) = resolve_gradle_sources_jar(cache, &coord)
        .expect("sources jar lookup should succeed");
    assert_eq!(resolved_version, "3.27.7");
    assert_eq!(jar, expected);
}

#[test]
fn resolve_gradle_sources_jar_falls_back_to_largest_version() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();
    make_gradle_cache_entry(
        cache,
        "org.example",
        "lib",
        "1.0.0",
        "lib-1.0.0-sources.jar",
    );
    let expected_newer = make_gradle_cache_entry(
        cache,
        "org.example",
        "lib",
        "2.0.0",
        "lib-2.0.0-sources.jar",
    );

    let coord = MavenCoord {
        group_id: "org.example".to_string(),
        artifact_id: "lib".to_string(),
        version: None,
    };
    let (resolved_version, jar) = resolve_gradle_sources_jar(cache, &coord).unwrap();
    assert_eq!(resolved_version, "2.0.0");
    assert_eq!(jar, expected_newer);
}

#[test]
fn resolve_gradle_sources_jar_returns_none_when_only_binary_jar() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();
    // Only the binary jar is present — common Gradle default; sources are
    // a separate dev-machine prereq we explicitly do NOT fall back from.
    make_gradle_cache_entry(
        cache,
        "org.example",
        "lib",
        "1.0.0",
        "lib-1.0.0.jar",
    );

    let coord = MavenCoord {
        group_id: "org.example".to_string(),
        artifact_id: "lib".to_string(),
        version: Some("1.0.0".to_string()),
    };
    assert!(resolve_gradle_sources_jar(cache, &coord).is_none());
}

#[test]
fn resolve_gradle_sources_jar_returns_none_for_unknown_artifact() {
    let tmp = TempDir::new().unwrap();
    let coord = MavenCoord {
        group_id: "org.nope".to_string(),
        artifact_id: "missing".to_string(),
        version: Some("1.0.0".to_string()),
    };
    assert!(resolve_gradle_sources_jar(tmp.path(), &coord).is_none());
}
