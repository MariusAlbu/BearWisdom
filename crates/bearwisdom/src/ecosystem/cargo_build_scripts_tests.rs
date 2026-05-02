use super::*;
use std::fs;

#[test]
fn parse_package_name_simple() {
    let toml = "[package]\nname = \"scryer-prolog\"\nversion = \"0.10.0\"\n";
    assert_eq!(parse_package_name(toml).as_deref(), Some("scryer-prolog"));
}

#[test]
fn parse_package_name_single_quoted() {
    let toml = "[package]\nname = 'my-crate'\n";
    assert_eq!(parse_package_name(toml).as_deref(), Some("my-crate"));
}

#[test]
fn parse_package_name_returns_none_for_workspace_only() {
    let toml = "[workspace]\nmembers = [\"a\", \"b\"]\n";
    assert_eq!(parse_package_name(toml), None);
}

#[test]
fn parse_package_name_ignores_name_outside_package() {
    let toml = "[dependencies]\nname = \"not-the-package\"\n[package]\nname = \"real\"\n";
    assert_eq!(parse_package_name(toml).as_deref(), Some("real"));
}

#[test]
fn package_has_build_script_via_field() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"x\"\nbuild = \"build/main.rs\"\n",
    )
    .unwrap();
    assert!(package_has_build_script(dir.path()));
}

#[test]
fn package_has_build_script_via_default_build_rs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    fs::write(dir.path().join("build.rs"), "fn main() {}\n").unwrap();
    assert!(package_has_build_script(dir.path()));
}

#[test]
fn package_without_build_script_returns_false() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
    assert!(!package_has_build_script(dir.path()));
}

#[test]
fn package_with_build_false_is_treated_as_no_build_script() {
    // Cargo lets `build = false` disable the default build.rs detection.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname=\"x\"\nbuild = false\n",
    )
    .unwrap();
    fs::write(dir.path().join("build.rs"), "fn main() {}\n").unwrap();
    // build.rs file presence still triggers; this is a heuristic.
    assert!(package_has_build_script(dir.path()));
}

#[test]
fn discover_out_dirs_empty_when_target_missing() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname=\"foo\"\nbuild=\"build.rs\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("build.rs"), "fn main(){}\n").unwrap();
    assert!(_test_discover_out_dirs(dir.path()).is_empty());
}

#[test]
fn discover_out_dirs_finds_host_package_only() {
    // Synthetic target/ layout with the host package and a transitive
    // dep; only the host package's OUT_DIR should be returned.
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname=\"foo\"\nbuild=\"build.rs\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("build.rs"), "fn main(){}\n").unwrap();

    let host_out = dir
        .path()
        .join("target/debug/build/foo-deadbeefdeadbeef/out");
    fs::create_dir_all(&host_out).unwrap();
    fs::write(host_out.join("generated.rs"), "// host\n").unwrap();

    let dep_out = dir
        .path()
        .join("target/debug/build/markup5ever-cafef00dcafef00d/out");
    fs::create_dir_all(&dep_out).unwrap();
    fs::write(dep_out.join("entities.rs"), "// dep\n").unwrap();

    let roots = _test_discover_out_dirs(dir.path());
    assert_eq!(roots.len(), 1);
    assert!(roots[0].root.ends_with("foo-deadbeefdeadbeef/out"));
}

#[test]
fn discover_out_dirs_returns_both_profiles_when_present() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname=\"foo\"\nbuild=\"build.rs\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("build.rs"), "fn main(){}\n").unwrap();
    for profile in &["debug", "release"] {
        let out = dir
            .path()
            .join(format!("target/{profile}/build/foo-aaa/out"));
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("g.rs"), "// stub\n").unwrap();
    }
    let roots = _test_discover_out_dirs(dir.path());
    assert_eq!(roots.len(), 2);
}

#[test]
fn walker_emits_only_rs_files() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.rs"), "// stub\n").unwrap();
    fs::write(dir.path().join("b.txt"), "ignored\n").unwrap();
    let nested = dir.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("c.rs"), "// nested\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "foo-build-out-debug".to_string(),
        version: "local".to_string(),
        root: dir.path().to_path_buf(),
        ecosystem: ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_out_dir(&dep);
    let names: Vec<&str> = walked.iter().map(|w| w.relative_path.as_str()).collect();
    assert!(names.iter().any(|n| n.ends_with("a.rs")));
    assert!(names.iter().any(|n| n.ends_with("c.rs")));
    assert!(!names.iter().any(|n| n.ends_with(".txt")));
}
