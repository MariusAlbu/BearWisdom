use super::*;
use std::fs;

fn make_r_src_fixture(root: &Path, packages: &[(&str, &[&str])]) {
    let library = root.join("src").join("library");
    fs::create_dir_all(&library).unwrap();
    for (pkg, files) in packages {
        let pkg_r = library.join(pkg).join("R");
        fs::create_dir_all(&pkg_r).unwrap();
        for fname in *files {
            fs::write(pkg_r.join(fname), "# stub\n").unwrap();
        }
    }
}

#[test]
fn walk_yields_r_files_per_base_package() {
    let tmp = tempfile::tempdir().unwrap();
    make_r_src_fixture(
        tmp.path(),
        &[
            ("base", &["zzz.R", "library.R"]),
            ("stats", &["lm.R"]),
            // Non-base package — should be skipped.
            ("dplyr", &["filter.R"]),
        ],
    );

    let dep = ExternalDepRoot {
        module_path: "r-stdlib".into(),
        version: String::new(),
        root: tmp.path().join("src").join("library"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_r_tree(&dep);
    let names: Vec<&str> = walked
        .iter()
        .map(|w| w.absolute_path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"zzz.R"));
    assert!(names.contains(&"library.R"));
    assert!(names.contains(&"lm.R"));
    assert!(!names.contains(&"filter.R"), "non-base package must be skipped");
    for w in &walked {
        assert_eq!(w.language, "r");
        assert!(w.relative_path.starts_with("ext:r-stdlib:"));
    }
}

#[test]
fn discover_returns_empty_without_env_var() {
    // Make sure no leftover var from another test leaks in.
    std::env::remove_var("BEARWISDOM_R_SRC");
    let roots = discover_r_stdlib();
    assert!(roots.is_empty());
}

#[test]
fn discover_returns_empty_when_path_lacks_src_library() {
    let tmp = tempfile::tempdir().unwrap();
    // Tmp dir exists but has no src/library/ child.
    std::env::set_var("BEARWISDOM_R_SRC", tmp.path());
    let roots = discover_r_stdlib();
    std::env::remove_var("BEARWISDOM_R_SRC");
    assert!(roots.is_empty());
}

#[test]
fn discover_returns_one_root_with_valid_r_src() {
    let tmp = tempfile::tempdir().unwrap();
    make_r_src_fixture(tmp.path(), &[("base", &["zzz.R"])]);

    std::env::set_var("BEARWISDOM_R_SRC", tmp.path());
    let roots = discover_r_stdlib();
    std::env::remove_var("BEARWISDOM_R_SRC");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "r-stdlib");
    assert!(roots[0].root.ends_with("src/library") || roots[0].root.ends_with("src\\library"));
}

#[test]
fn walk_returns_empty_when_root_missing() {
    let dep = ExternalDepRoot {
        module_path: "r-stdlib".into(),
        version: String::new(),
        root: PathBuf::from("/__no_such_r_src_for_test__/zzz"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    assert!(walk_r_tree(&dep).is_empty());
}
