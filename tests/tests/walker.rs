//! Integration tests for the file walker.

use bearwisdom::walker::{detect_language, walk};
use bearwisdom_tests::TestProject;

#[test]
fn walk_discovers_all_source_files() {
    let project = TestProject::csharp_service();
    let files = walk(project.path()).unwrap();

    assert!(files.len() >= 4, "expected at least 4 C# files, got {}", files.len());

    for f in &files {
        assert!(f.relative_path.ends_with(".cs"), "unexpected file: {}", f.relative_path);
        assert_eq!(f.language, "csharp");
    }
}

#[test]
fn walk_multi_language() {
    let project = TestProject::multi_lang();
    let files = walk(project.path()).unwrap();

    let languages: std::collections::HashSet<&str> = files.iter().map(|f| f.language).collect();
    assert!(languages.contains("csharp"), "should find C# files");
    assert!(languages.contains("python"), "should find Python files");
    assert!(languages.contains("typescript"), "should find TypeScript files");
}

#[test]
fn walk_returns_sorted_paths() {
    let project = TestProject::csharp_service();
    let files = walk(project.path()).unwrap();

    let paths: Vec<&str> = files.iter().map(|f| f.relative_path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "walked files should be sorted by path");
}

#[test]
fn walk_empty_directory() {
    let dir = tempfile::TempDir::new().unwrap();
    let files = walk(dir.path()).unwrap();
    assert!(files.is_empty());
}

#[test]
fn walk_ignores_non_source_files() {
    let project = TestProject::csharp_service();
    // Add non-source files that should be ignored.
    project.add_file("README.md", "# Hello");
    project.add_file("data.csv", "a,b,c");
    project.add_file(".gitignore", "target/");

    let files = walk(project.path()).unwrap();

    for f in &files {
        assert!(
            detect_language(std::path::Path::new(&f.relative_path)).is_some(),
            "walker returned a non-source file: {}",
            f.relative_path,
        );
    }
}

#[test]
fn detect_language_common_extensions() {
    let cases = [
        ("main.rs", Some("rust")),
        ("app.py", Some("python")),
        ("index.ts", Some("typescript")),
        ("style.css", Some("css")),
        ("data.json", Some("json")),
        ("notes.txt", None),
    ];

    for (filename, expected) in cases {
        let result = detect_language(std::path::Path::new(filename));
        assert_eq!(
            result, expected,
            "detect_language({filename}) = {result:?}, expected {expected:?}",
        );
    }
}

#[test]
fn walk_excludes_build_directories() {
    let project = TestProject::typescript_app();
    // Simulate build output that should be ignored.
    project.add_file("node_modules/lodash/index.js", "module.exports = {};");
    project.add_file("dist/bundle.js", "// compiled");

    let files = walk(project.path()).unwrap();

    for f in &files {
        assert!(
            !f.relative_path.starts_with("node_modules"),
            "walker should exclude node_modules: {}",
            f.relative_path,
        );
    }
}
