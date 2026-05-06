use super::*;
use std::fs;

#[test]
fn looks_like_perl_version_matches_versioned_dirs() {
    assert!(looks_like_perl_version(Path::new("5.30")));
    assert!(looks_like_perl_version(Path::new("5.36.0")));
    assert!(looks_like_perl_version(Path::new("5.40")));
    assert!(!looks_like_perl_version(Path::new("vendor_perl")));
    assert!(!looks_like_perl_version(Path::new("auto")));
    assert!(!looks_like_perl_version(Path::new("share")));
}

#[test]
fn walk_finds_pm_files_skips_test_dirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    fs::write(root.join("Carp.pm"), "package Carp;\n1;\n").unwrap();
    fs::create_dir_all(root.join("Data")).unwrap();
    fs::write(root.join("Data/Dumper.pm"), "package Data::Dumper;\n1;\n").unwrap();
    // Test dir should be excluded.
    fs::create_dir_all(root.join("t")).unwrap();
    fs::write(root.join("t/Carp.t"), "use Carp;\n").unwrap();
    fs::write(root.join("t/excluded.pm"), "package Excluded;\n1;\n").unwrap();
    // auto/ dir should be excluded.
    fs::create_dir_all(root.join("auto/Carp")).unwrap();
    fs::write(root.join("auto/Carp/Carp.bs"), "binary").unwrap();

    let dep = ExternalDepRoot {
        module_path: "perl-stdlib".into(),
        version: String::new(),
        root: root.clone(),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_perl_tree(&dep);
    let names: Vec<&str> = walked
        .iter()
        .map(|w| w.absolute_path.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"Carp.pm"));
    assert!(names.contains(&"Dumper.pm"));
    assert!(!names.contains(&"excluded.pm"), "t/ subdir should be excluded");
    for w in &walked {
        assert_eq!(w.language, "perl");
        assert!(w.relative_path.starts_with("ext:perl-stdlib:"));
    }
}

#[test]
fn walk_returns_empty_when_dir_missing() {
    let dep = ExternalDepRoot {
        module_path: "perl-stdlib".into(),
        version: String::new(),
        root: PathBuf::from("/__no_such_perl_dir_for_test__/zzz"),
        ecosystem: TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    assert!(walk_perl_tree(&dep).is_empty());
}

#[test]
fn discover_with_explicit_env_override() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("Carp.pm"), "package Carp;\n1;\n").unwrap();

    // SAFETY: we set BEARWISDOM_PERL_STDLIB for the duration of this test.
    // Concurrent tests touching the same env var would race, but we use a
    // unique-per-test path so the value is stable across reads.
    std::env::set_var("BEARWISDOM_PERL_STDLIB", tmp.path());
    let roots = discover_perl_stdlib();
    std::env::remove_var("BEARWISDOM_PERL_STDLIB");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].root, tmp.path());
    assert_eq!(roots[0].module_path, "perl-stdlib");
}
