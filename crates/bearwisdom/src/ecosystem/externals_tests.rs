use std::fs;

use tempfile::TempDir;

use super::{
    pick_newest_version, resolve_coursier_sources_jar, resolve_coursier_submodule_jars,
    resolve_gradle_sources_jar, strip_scala_suffix,
};
use crate::ecosystem::manifest::maven::MavenCoord;

#[test]
fn pick_newest_version_handles_double_digit_components() {
    // Lexicographic sort would give "3.9.1" because '9' > '1'; semver
    // ordering must give "3.12.0".
    let versions = vec![
        "3.3.0".to_string(),
        "3.5.0".to_string(),
        "3.9.1".to_string(),
        "3.10.2".to_string(),
        "3.11.0".to_string(),
        "3.12.0".to_string(),
    ];
    assert_eq!(pick_newest_version(&versions).as_deref(), Some("3.12.0"));
}

#[test]
fn pick_newest_version_release_beats_pre_release() {
    let versions = vec!["1.0.0-RC1".to_string(), "1.0.0".to_string()];
    assert_eq!(pick_newest_version(&versions).as_deref(), Some("1.0.0"));
}

#[test]
fn pick_newest_version_orders_milestones_lex() {
    let versions = vec![
        "1.0.0-M9".to_string(),
        "1.0.0-M43".to_string(),
        "1.0.0-M38".to_string(),
    ];
    // Same numeric prefix, qualifier comparison falls back to lex —
    // "M9" > "M43" lex-wise. Documenting the actual behavior so the
    // semver-snobs callers know to pin explicit versions for milestones.
    assert_eq!(
        pick_newest_version(&versions).as_deref(),
        Some("1.0.0-M9")
    );
}

#[test]
fn pick_newest_version_empty_returns_none() {
    let versions: Vec<String> = Vec::new();
    assert!(pick_newest_version(&versions).is_none());
}

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

/// Build a fake Coursier cache layout under `root`:
///   <root>/v1/https/<host>/<repo-base>/<group-as-path>/<artifact>/<version>/<file>
fn make_coursier_cache_entry(
    root: &std::path::Path,
    host: &str,
    repo_base: &[&str],
    group: &str,
    artifact: &str,
    version: &str,
    file_name: &str,
) -> std::path::PathBuf {
    let mut dir = root.join("v1").join("https").join(host);
    for seg in repo_base {
        dir = dir.join(seg);
    }
    for seg in group.split('.') {
        dir = dir.join(seg);
    }
    dir = dir.join(artifact).join(version);
    fs::create_dir_all(&dir).unwrap();
    let f = dir.join(file_name);
    fs::write(&f, b"").unwrap();
    f
}

#[test]
fn resolve_coursier_sources_jar_finds_central_layout() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();
    let expected = make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
        "co.fs2",
        "fs2-core_3",
        "3.12.0",
        "fs2-core_3-3.12.0-sources.jar",
    );

    let coord = MavenCoord {
        group_id: "co.fs2".to_string(),
        artifact_id: "fs2-core_3".to_string(),
        version: Some("3.12.0".to_string()),
    };
    let (version, jar) =
        resolve_coursier_sources_jar(cache, &coord).expect("Coursier lookup");
    assert_eq!(version, "3.12.0");
    assert_eq!(jar, expected);
}

#[test]
fn resolve_coursier_sources_jar_descends_through_alt_repo() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();
    // Coursier sometimes pulls from oss.sonatype.org instead of repo1.
    let expected = make_coursier_cache_entry(
        cache,
        "oss.sonatype.org",
        &["content", "repositories", "snapshots"],
        "com.twitter",
        "util-core_2.13",
        "23.11.0",
        "util-core_2.13-23.11.0-sources.jar",
    );

    let coord = MavenCoord {
        group_id: "com.twitter".to_string(),
        artifact_id: "util-core_2.13".to_string(),
        version: Some("23.11.0".to_string()),
    };
    let (_v, jar) = resolve_coursier_sources_jar(cache, &coord).unwrap();
    assert_eq!(jar, expected);
}

#[test]
fn resolve_coursier_sources_jar_falls_back_to_largest_version() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();
    make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
        "org.example",
        "lib",
        "1.0.0",
        "lib-1.0.0-sources.jar",
    );
    let expected = make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
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
    let (version, jar) = resolve_coursier_sources_jar(cache, &coord).unwrap();
    assert_eq!(version, "2.0.0");
    assert_eq!(jar, expected);
}

#[test]
fn resolve_coursier_sources_jar_returns_none_for_missing_artifact() {
    let tmp = TempDir::new().unwrap();
    let coord = MavenCoord {
        group_id: "org.nope".to_string(),
        artifact_id: "missing".to_string(),
        version: Some("1.0.0".to_string()),
    };
    assert!(resolve_coursier_sources_jar(tmp.path(), &coord).is_none());
}

#[test]
fn strip_scala_suffix_removes_version_suffixes() {
    assert_eq!(strip_scala_suffix("scalatest_2.13"), "scalatest");
    assert_eq!(strip_scala_suffix("cats-core_3"), "cats-core");
    assert_eq!(strip_scala_suffix("scalatest-core_2.12"), "scalatest-core");
    assert_eq!(strip_scala_suffix("scalatest-compatible"), "scalatest-compatible");
    assert_eq!(strip_scala_suffix("akka-actor_2.13"), "akka-actor");
}

#[test]
fn resolve_coursier_submodule_jars_finds_split_modules() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();

    // Aggregator artifact — the jar BW initially resolves.
    make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
        "org.scalatest",
        "scalatest_2.13",
        "3.2.20",
        "scalatest_2.13-3.2.20-sources.jar",
    );
    // Sub-module sources jars under the same Coursier group dir.
    let core_jar = make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
        "org.scalatest",
        "scalatest-core_2.13",
        "3.2.20",
        "scalatest-core_2.13-3.2.20-sources.jar",
    );
    let matchers_jar = make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
        "org.scalatest",
        "scalatest-shouldmatchers_2.13",
        "3.2.20",
        "scalatest-shouldmatchers_2.13-3.2.20-sources.jar",
    );

    let subs = resolve_coursier_submodule_jars(cache, "org.scalatest", "scalatest", Some("3.2.20"));
    let found_artifacts: Vec<&str> = subs.iter().map(|(a, _, _)| a.as_str()).collect();

    // Both sub-modules found; aggregator itself excluded.
    assert!(found_artifacts.contains(&"scalatest-core_2.13"), "missing core");
    assert!(found_artifacts.contains(&"scalatest-shouldmatchers_2.13"), "missing matchers");
    assert!(!found_artifacts.contains(&"scalatest_2.13"), "aggregator must be excluded");

    // Verify the jar paths are correct.
    let core_found = subs.iter().find(|(a, _, _)| a == "scalatest-core_2.13").unwrap();
    assert_eq!(core_found.2, core_jar);
    let matchers_found = subs.iter().find(|(a, _, _)| a == "scalatest-shouldmatchers_2.13").unwrap();
    assert_eq!(matchers_found.2, matchers_jar);
}

#[test]
fn resolve_coursier_submodule_jars_empty_when_no_siblings() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path();

    // Only the aggregator, no sub-modules.
    make_coursier_cache_entry(
        cache,
        "repo1.maven.org",
        &["maven2"],
        "com.example",
        "mylib_2.13",
        "1.0.0",
        "mylib_2.13-1.0.0-sources.jar",
    );

    let subs = resolve_coursier_submodule_jars(cache, "com.example", "mylib", Some("1.0.0"));
    assert!(subs.is_empty(), "no sub-modules should be found");
}
